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
use std::io::{IsTerminal as _, Write};
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

use loader::{Answer, Binding, EndingAsk, PeekAsk, ReadAsk, RecordsAsk, Request, Wake};
use panel::{Opening, Orch, PanelByte};
use peek::Body;

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
pub mod model;
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
#[cfg(test)]
mod s5_app;
pub mod select;
#[cfg(test)]
mod select_spec;
mod settings;
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

/// What the open reader holds of its file, pinned to the model's generation:
/// an answer that landed for another is dropped.
#[derive(Debug, Default)]
struct ReaderPane {
    generation: u64,
    /// The file as the link named it, and the line it asked for.
    path: String,
    line: Option<usize>,
    /// The turn's file occurrences, with the active position and resolution base.
    links: usize,
    files: Vec<link::FileRef>,
    base: link::Base,
    index: usize,
    /// The body on screen; `None` until the answer lands.
    body: Option<reader::Body>,
    /// The body's wrap, and the text width it was made for.
    flow: Option<(usize, reader::Flow)>,
    /// The `:line` jump waits for the first frame that wraps the body.
    jump: bool,
    /// Markdown keeps its parse across widths and both views across toggles.
    document: Option<markdown::Document>,
    rendered: Option<(usize, markdown::Rendered)>,
    raw: bool,
    /// A mode or width change restores this zero-based source anchor.
    anchor: Option<usize>,
}

impl ReaderPane {
    fn shown(&self) -> Option<markdown::Shown<'_>> {
        let (_, rendered) = self.rendered.as_ref()?;
        Some(if self.raw || rendered.refusal().is_some() {
            markdown::Shown::Raw(&self.flow.as_ref()?.1)
        } else {
            markdown::Shown::Rendered(rendered)
        })
    }

    fn source_line(&self, top: usize) -> usize {
        match self.shown() {
            Some(markdown::Shown::Rendered(r)) => r.line_of(top.min(r.rows().saturating_sub(1))),
            _ => self
                .flow
                .as_ref()
                .and_then(|(_, f)| f.display(top.min(f.rows().saturating_sub(1))).map(|r| r.0))
                .unwrap_or(0),
        }
    }

    fn reflow(&mut self, width: usize, top: usize) {
        let source = self.source_line(top);
        let Some(reader::Body::Text { lines, .. }) = &self.body else {
            return;
        };
        let room = width.saturating_sub(reader::gutter(lines.len()));
        if self.flow.as_ref().is_none_or(|(held, _)| *held != room) {
            self.flow = Some((room, reader::flow(lines, room)));
        }
        if let Some(doc) = &self.document
            && self
                .rendered
                .as_ref()
                .is_none_or(|(held, _)| *held != width)
        {
            if self.rendered.is_some() {
                self.anchor.get_or_insert(source);
            }
            self.rendered = Some((width, doc.layout(width)));
        }
    }
}

/// What a press on a link cell does when its un-dragged release lands there.
#[derive(Debug)]
enum Press {
    /// Open this file occurrence in the reader.
    File(draw::FileHit),
    /// Show the hint that names the terminal's link modifier.
    Web,
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
    /// The last mouse input of the drag.
    touched: Instant,
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
                if self.composing() {
                    self.write(false);
                }
                if self.model.reader_open() {
                    self.close_reader();
                }
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

    /// A click on a seat: inside tmux the peek panel; outside tmux the seat's
    /// own pane, interactive, through [`App::ask_pane_view`].
    fn open_seat(&mut self) {
        if self.server.is_some() {
            self.open_peek();
            return;
        }
        let Some(session) = self.model.selected().map(str::to_owned) else {
            let line = "refused: view: no session is selected".to_owned();
            self.flash = Some((line, Instant::now()));
            return;
        };
        let drawn = self.drawn.clone().filter(|drawn| drawn.session == session);
        let label = (drawn.as_ref())
            .and_then(|drawn| drawn.focused.clone())
            .unwrap_or_else(|| session.clone());
        self.ask_pane_view(session, drawn.as_ref(), label);
    }

    /// Ask the loop to hand the terminal to the drawn seat's pane, or say why
    /// not on the hint row.
    fn ask_pane_view(&mut self, session: String, drawn: Option<&Drawn>, label: String) {
        let refuse = |why: String| (format!("refused: view {label}: {why}"), Instant::now());
        if (self.entry(&session)).is_some_and(|entry| entry.status == Status::Stopped) {
            self.flash = Some(refuse(Refusal::Stopped(session).why()));
            return;
        }
        let Some(dir) = self.dirs.get(&session).cloned() else {
            self.flash = Some(refuse(format!(
                "{session} has no record directory ae can read"
            )));
            return;
        };
        let Some(drawn) = drawn else {
            self.flash = Some(refuse("the view changed".to_owned()));
            return;
        };
        let seat = match drawn.seat() {
            Ok(seat) => seat.clone(),
            Err(why) => {
                self.flash = Some(refuse(why.why()));
                return;
            }
        };
        if self.composing() {
            self.write(false);
        }
        if self.model.reader_open() {
            self.close_reader();
        }
        self.pane_view = Some(paneview::Ask {
            session,
            uuid: drawn.uuid.clone(),
            dir,
            seat,
            label,
        });
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

    /// Fold the file the open reader asked for. Only the open reader's own
    /// generation lands: an answer for a closed or an older open is dropped.
    fn read_landed(&mut self, generation: u64, body: reader::Body) {
        if self.model.reader_generation() != Some(generation)
            || self.reader.generation != generation
        {
            return;
        }
        self.reader.document = match &body {
            reader::Body::Text { lines, .. } if markdown::is_markdown(&self.reader.path) => {
                Some(markdown::Document::parse(&lines.join("\n")))
            }
            _ => None,
        };
        self.reader.rendered = None;
        self.reader.anchor = None;
        self.reader.body = Some(body);
        self.reader.flow = None;
        self.reader.jump = true;
    }

    /// The cache and lane width a turn's links are read with, the ones the last
    /// frame drew it with.
    fn md(&self) -> (&RefCell<lanemd::Cache>, usize) {
        (&self.md, usize::from(self.layout.lane_width))
    }

    /// `r`: open the reader on the first file link of the marked turn, read as
    /// the displayed lane parts, in the session's link base. A refusal is said
    /// on the hint row, opens nothing and asks the reader thread nothing.
    fn open_reader(&mut self) {
        let item = match self.marked_item() {
            Ok(item) => item,
            Err(words) => {
                self.flash = Some((words, Instant::now()));
                return;
            }
        };
        let base = self.overview.base.clone();
        let found = self
            .keys()
            .iter()
            .position(|key| *key == lane::key(&item))
            .map(|at| lane::read_links(&self.lane.items, at, &base, Some(self.md())).1)
            .unwrap_or_default();
        self.read_file(found, 0, base);
    }

    /// Every activation and step uses the same reader generation and file door.
    fn read_file(&mut self, found: Vec<link::FileRef>, index: usize, base: link::Base) {
        let Some(first) = found.get(index) else {
            self.flash = Some(("no file link in that turn".to_owned(), Instant::now()));
            return;
        };
        let ask = Box::new(ReadAsk {
            generation: self.reader_generation.wrapping_add(1),
            path: first.path.clone(),
            line: first.line,
            links: found.len(),
            base,
        });
        self.reader_generation = ask.generation;
        self.model.open_reader(ask.generation);
        // A visual file is not read: the panel holds the note at once, and the
        // reader thread is told to drop any ask for the file left behind.
        let visual = reader::visual(&ask.path);
        self.reader = ReaderPane {
            generation: ask.generation,
            path: ask.path.clone(),
            line: ask.line,
            links: ask.links,
            files: found,
            base: ask.base.clone(),
            index,
            body: visual.map(|kind| reader::Body::Gap(reader::visual_note(kind))),
            raw: ask.line.is_some(),
            ..ReaderPane::default()
        };
        if let Some(tx) = &self.ask {
            let _ = tx.send(Request::Read(visual.is_none().then_some(ask)));
        }
    }

    /// Resolve the press frame against the current turn before using its ordinal.
    fn file_list(&mut self, hit: &draw::FileHit) -> Option<Vec<link::FileRef>> {
        if self.model.selected() != Some(hit.session.as_str())
            || self.overview.base != hit.link.base
        {
            return None;
        }
        let at = self.keys().iter().position(|key| *key == hit.link.turn)?;
        if self.fold_key(self.lane.items.get(at)?) != hit.fold {
            return None;
        }
        let (fingerprint, files) =
            lane::read_links(&self.lane.items, at, &hit.link.base, Some(self.md()));
        (fingerprint == hit.link.fingerprint && files.get(hit.link.index) == Some(&hit.link.file))
            .then_some(files)
    }

    fn changed_read(&mut self) {
        self.flash = Some(("refused: read: the view changed".to_owned(), Instant::now()));
    }

    /// Close the reader and tell the reader thread to drop an unread ask. It
    /// always tells: the model may already hold the close the key caused.
    fn close_reader(&mut self) {
        self.model.close_reader();
        self.reader = ReaderPane::default();
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Read(None));
        }
    }

    /// A mouse event with a seat viewed: the sidebar stays live — a press
    /// on the viewed session returns to its chat, one on another session
    /// or seat goes through the ordinary press (which closes or retargets
    /// the view) — the wheel scrolls what it lands on, and a press on the
    /// viewed body begins a selection.
    fn peek_mouse(&mut self, mouse: Mouse, fresh: bool) -> bool {
        self.panel_mouse(mouse, fresh, false)
    }

    /// [`App::peek_mouse`] for the reader, which takes the same presses: its
    /// column has no close label, and the wheel scrolls the file.
    fn reader_mouse(&mut self, mouse: Mouse, fresh: bool) -> bool {
        self.panel_mouse(mouse, fresh, true)
    }

    /// Fold one settings answer. Only the open overlay's own generation
    /// paints: a late answer for a closed overlay is dropped. Quota rides
    /// every answer; config, about and instructions ride the first per open,
    /// then stay.
    fn settled(
        &mut self,
        generation: u64,
        quota: Vec<crate::quota::view::Scope>,
        config: Option<settings::ConfigView>,
        about: Option<settings::AboutFacts>,
        instructions: Option<settings::InstructionsView>,
    ) {
        if !self.model.settings_generation(generation) {
            return;
        }
        // A refresh that failed outright keeps the body already held, marked.
        let held = (self.settings_bodies.quota.as_ref())
            .is_some_and(|held| *held != crate::quota::view::unavailable());
        if quota == crate::quota::view::unavailable() && held && self.quota_at.is_some() {
            self.settings_bodies.quota_kept = Some(String::new());
            self.quota_failed = true;
        } else {
            self.settings_bodies.quota = Some(quota);
            self.settings_bodies.quota_kept = None;
            self.quota_at = Some(Timestamp::now().epoch());
            self.quota_failed = false;
        }
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
        // An earlier open's quota stays drawn, marked kept, until this open's own answer.
        let quota = self.settings_bodies.quota.take();
        let live_seats = self.settings_bodies.live_seats.take();
        self.settings_bodies = settings::SettingsBodies {
            quota_kept: quota.as_ref().map(|_| String::new()),
            quota,
            live_seats,
            ..settings::SettingsBodies::default()
        };
        self.quota_failed = false;
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Settings {
                open: true,
                generation: self.settings_generation,
            });
        }
    }

    /// Spell a kept quota body's line from the one epoch, so its age moves with the clock.
    fn spell_kept(&mut self) {
        if let (Some(kept), Some(at)) = (&mut self.settings_bodies.quota_kept, self.quota_at) {
            let age = crate::quota::age_label(Timestamp::now().epoch().saturating_sub(at));
            *kept = if self.quota_failed {
                format!("refresh failed · showing values from {age} ago")
            } else {
                format!("last updated {age} — refreshing")
            };
        }
    }

    /// What the overlay owes before its frame.
    fn settings_due(&mut self) {
        self.spell_kept();
        self.ask_stats();
        self.ask_tools();
    }

    /// Tell the reader, before a frame, when the Statistics tab became shown
    /// or stopped being shown under the open overlay: shown, it watches the
    /// store; hidden, it reads nothing for it. A repaint asks nothing.
    fn ask_stats(&mut self) {
        let shown = (self.model.settings())
            .filter(|open| open.tab == model::SettingsTab::Statistics)
            .map(|open| open.generation);
        if shown == self.stats_shown {
            return;
        }
        let (generation, now) = match (shown, self.stats_shown) {
            (Some(generation), _) => (generation, true),
            (None, Some(generation)) => (generation, false),
            (None, None) => return,
        };
        self.stats_shown = shown;
        self.ask_stats_for(generation, now);
    }

    /// Tell the reader the Statistics tab of `generation` is `shown` or not.
    fn ask_stats_for(&self, generation: u64, shown: bool) {
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Stats { generation, shown });
        }
    }

    /// What the store shows, for the open it was read for and no other. A
    /// reset's line stays until the next reset or the overlay closes.
    fn stats_landed(&mut self, generation: u64, glance: &crate::stats_store::disk::Glance) {
        if !self.model.settings_generation(generation) {
            return;
        }
        let (view, mut store) = settings::StoreLine::of(glance);
        let stats = &mut self.settings_bodies.stats;
        store.reset = stats.store.reset.take();
        stats.view = view;
        stats.store = store;
    }

    /// `r` on the Statistics tab: a reset owed to the upkeep, for this open.
    fn ask_reset(&mut self) {
        // The store reset belongs to the Spawns page alone.
        if self.settings_bodies.stats.page != settings::StatsPage::Spawns {
            return;
        }
        let shown = (self.model.settings())
            .filter(|open| open.tab == model::SettingsTab::Statistics)
            .map(|open| open.generation);
        if let Some(generation) = shown {
            self.stats_reset = Some(generation);
            self.settings_bodies.stats.store.reset = Some(settings::ResetLine::Requested);
        }
    }

    /// The reset owed to the upkeep, handed over once.
    const fn take_reset(&mut self) -> Option<u64> {
        self.stats_reset.take()
    }

    /// No upkeep thread took the reset of `generation`: it waits for one.
    fn reset_waits(&mut self, generation: u64) {
        if self.model.settings_generation(generation) {
            self.settings_bodies.stats.store.reset = Some(settings::ResetLine::Waiting);
        }
    }

    /// What became of the reset of `generation`, drawn only under that open.
    /// A reset that took drops the old numbers, and the watch reads the
    /// store again at once.
    fn reset_landed(
        &mut self,
        generation: u64,
        outcome: Result<crate::stats_store::disk::Reset, crate::stats_store::disk::Refused>,
    ) {
        use crate::stats_store::disk::Reset;
        use settings::{ResetLine, ScopeLine};
        if !self.model.settings_generation(generation) {
            return;
        }
        let stats = &mut self.settings_bodies.stats;
        let line = match outcome {
            Ok(Reset::Rebuilt { .. } | Reset::Empty) => {
                stats.view = None;
                stats.store.all = ScopeLine::Building;
                stats.store.this = ScopeLine::Building;
                if self.stats_shown == Some(generation) {
                    self.ask_stats_for(generation, true);
                }
                ResetLine::Rebuilt
            }
            Ok(Reset::Busy) => ResetLine::Busy,
            Err(why) => ResetLine::Refused(why.describe()),
        };
        self.settings_bodies.stats.store.reset = Some(line);
    }

    /// Show the other scope of the read already held.
    fn flip_stats_scope(&mut self) {
        let stats = &mut self.settings_bodies.stats;
        stats.scope = stats.scope.flip();
        // A tool page waits for the other scope's own answer.
        if stats.page != settings::StatsPage::Spawns {
            stats.tools = None;
        }
    }

    /// Left or Right on the Statistics tab: the inner tab one step along the
    /// row, wrapping. See [`Self::select_stats_tab`].
    fn stats_tab(&mut self, next: bool) -> bool {
        let shown = settings::StatsTab::of(self.settings_bodies.stats.page);
        self.select_stats_tab(shown.step(next))
    }

    /// Show the inner tab `tab` on its first page unless it is the one shown
    /// (a click on the shown title changes nothing). A new tab draws
    /// `loading` with no filter until its own answer lands.
    fn select_stats_tab(&mut self, tab: settings::StatsTab) -> bool {
        let stats = &mut self.settings_bodies.stats;
        if settings::StatsTab::of(stats.page) == tab {
            return false;
        }
        stats.page = tab.first_page();
        stats.filter = crate::tool_stats::Filter::All;
        stats.tools = None;
        true
    }

    /// `n` or `p` on the Statistics tab: the next or previous page. A new
    /// page draws `loading` with no filter until its own answer lands.
    fn stats_page(&mut self, next: bool) -> bool {
        use settings::StatsPage;
        let stats = &mut self.settings_bodies.stats;
        let order = [
            StatsPage::Spawns,
            StatsPage::Skills,
            StatsPage::Mcp,
            StatsPage::Tools,
        ];
        let at = order
            .iter()
            .position(|page| *page == stats.page)
            .unwrap_or(0);
        let step = if next { 1 } else { order.len() - 1 };
        stats.page = order[(at + step) % order.len()];
        stats.filter = crate::tool_stats::Filter::All;
        stats.tools = None;
        true
    }

    /// `]` or `[` on a tool page: the next or previous filter the shown
    /// answer offers (`all` first). The numbers stay until the filtered
    /// answer lands.
    fn stats_filter(&mut self, next: bool) -> bool {
        let stats = &mut self.settings_bodies.stats;
        let Some(view) = &stats.tools else {
            return false;
        };
        let filters = &view.filters;
        if filters.is_empty() {
            return false;
        }
        let at = filters
            .iter()
            .position(|filter| *filter == stats.filter)
            .unwrap_or(0);
        let step = if next { 1 } else { filters.len() - 1 };
        stats.filter = filters[(at + step) % filters.len()].clone();
        true
    }

    /// The tool page the open Statistics tab shows, as the reader is asked
    /// for it: `None` on the Spawns page or with the tab not shown. `m` asks
    /// for this session: the selection's recorded id, else home's.
    fn tools_ask(&self) -> Option<(u64, crate::tool_stats::Ask)> {
        use crate::tool_stats::{Ask, Page, Scope};
        let open = self
            .model
            .settings()
            .filter(|open| open.tab == model::SettingsTab::Statistics)?;
        let stats = &self.settings_bodies.stats;
        let page = match stats.page {
            settings::StatsPage::Spawns => return None,
            settings::StatsPage::Skills => Page::Skills,
            settings::StatsPage::Mcp => Page::Mcp,
            settings::StatsPage::Tools => Page::Tools,
        };
        let scope = match stats.scope {
            settings::StatsScope::All => Scope::All,
            settings::StatsScope::ThisSession => {
                let name = self.model.selected().or(self.home.as_deref());
                Scope::This(name.and_then(|name| self.ids.get(name)).cloned())
            }
        };
        Some((
            open.generation,
            Ask {
                page,
                scope,
                filter: stats.filter.clone(),
            },
        ))
    }

    /// Tell the reader, before a frame, which tool page is shown, or that
    /// none is any more. A repaint asks nothing.
    fn ask_tools(&mut self) {
        let shown = self.tools_ask();
        if shown == self.tools_asked {
            return;
        }
        let request = match (&shown, &self.tools_asked) {
            (Some((generation, ask)), _) => Request::Tools {
                generation: *generation,
                shown: true,
                ask: ask.clone(),
            },
            (None, Some((generation, ask))) => Request::Tools {
                generation: *generation,
                shown: false,
                ask: ask.clone(),
            },
            (None, None) => return,
        };
        self.tools_asked = shown;
        if let Some(ask) = &self.ask {
            let _ = ask.send(request);
        }
    }

    /// A tool-statistics answer: kept only for the open overlay and the page
    /// it shows now (a stale or another page's answer is dropped).
    fn tools_landed(&mut self, generation: u64, view: Box<crate::tool_stats::View>) {
        if !self.model.settings_generation(generation) {
            return;
        }
        if self.tools_ask().is_some_and(|(_, ask)| ask == view.ask) {
            self.settings_bodies.stats.tools = Some(view);
        }
    }

    /// Close the overlay and tell the reader to stop reading for it. It
    /// always tells: the model may already hold the close the key caused.
    fn close_settings(&mut self) {
        self.model.close_settings();
        // The close below drops the reader's watches with the overlay.
        self.stats_shown = None;
        self.tools_asked = None;
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

    /// Fold the stopped sessions of the fleet just read into the sidebar's one
    /// group: out of the rows while it is closed, in them while it is open. A
    /// read opens it when its own selection, already read once, turned stopped,
    /// and when it takes a handed-over view of a stopped session: a session the
    /// human is on never goes missing from under them.
    fn fold_stopped(&mut self, seen: bool, scanned: bool) {
        let stopped =
            |name: &str| (self.fleet.rows.iter()).any(|row| row.name == name && row.is_stopped());
        let selected = seen && self.model.selected().is_some_and(stopped);
        let handed = self.keep.as_ref().is_some_and(|keep| {
            let name = keep.view.session.as_str();
            let id = self.ids.get(name).map(String::as_str);
            let now = Timestamp::now().epoch();
            stopped(name)
                && handover::decide(keep.at, now, &keep.view, true, id, scanned)
                    == handover::Verdict::Apply
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
    fn stopped(&self) -> model::Stopped {
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
    fn toggle_stopped(&mut self) -> bool {
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
        // The orchestrator's lane is read for its panel: open on that very
        // incarnation, else dropped.
        let panel = view.name == ORCH;
        let pinned =
            (self.orch.panel.as_ref()).is_some_and(|panel| panel.pin.as_ref() == Some(&view.id));
        if panel && !pinned && !home {
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
    fn evict(&mut self) {
        let rows: Vec<String> = self.fleet.rows.iter().map(|row| row.name.clone()).collect();
        let keep = loader::order(&rows, self.model.selected(), self.home.as_deref());
        let open = self.orch.panel.is_some();
        self.shown
            .retain(|name, _| open && name == ORCH || keep.iter().any(|(kept, _)| kept == name));
    }

    /// Tell the reader which session shows now, once per change.
    fn focus(&mut self) {
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

    /// A press acts on the frame on screen when it was made: one made before a
    /// frame replaced the layout is hit against the layout that frame replaced,
    /// so an ask in flight or a read since cannot move its target. A held wheel
    /// notch keeps replaying against the newest frame, which bounds it. One
    /// made on a frame the bound dropped is not taken, and the hint row says so.
    /// A right press opens the menu on the card its frame drew there.
    fn mouse_made(&mut self, mouse: Mouse, origin: Instant) -> bool {
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
        let acted = match self.panel_made(mouse, origin) {
            Some(acted) => acted,
            None if self.covered_since(mouse, origin) => self.frame_gone(),
            None => self.frame_made(mouse, origin),
        };
        acted || cleared
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
    fn frame_made(&mut self, mouse: Mouse, origin: Instant) -> bool {
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
    fn frame_gone(&mut self) -> bool {
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
        if mouse.kind != MouseKind::Release {
            self.read_press = None;
        }
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
        if self.model.reader_open() {
            return self.reader_mouse(mouse, fresh);
        }
        if !self.orch.active && self.on_panel(mouse) {
            return self.panel_route(mouse, fresh);
        }
        // A press beyond the drawn frame, before the redraw after a resize,
        // or on a frame below the size floor lands on no column and moves no
        // keys.
        let area = self.layout.area();
        let drawn = draw::floor_met(area) && area.contains(Position::new(mouse.column, mouse.row));
        if !self.orch.active && mouse.kind == MouseKind::Click && self.orch.panel.is_some() && drawn
        {
            self.focus_column(false);
            // A full panel kept the keys: the main column this press was aimed
            // at is covered now, so nothing there acts on it.
            if self.panel_focused() {
                return self.frame_gone();
            }
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
            MouseKind::Drag | MouseKind::Menu => ended,
        }
    }

    /// A mouse event with the overlay open: the wheel scrolls its body
    /// wherever it lands, a press on the gear or the close label closes, one
    /// on a tab title shows that tab, one on an inner title shows that inner
    /// tab, and any other press dies.
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
                Some(draw::Hit::StatsTab(tab)) => self.select_stats_tab(tab),
                _ => false,
            },
            MouseKind::Drag | MouseKind::Release | MouseKind::Menu => false,
        }
    }

    /// A press: what it always did, then a selection armed where it landed.
    fn clicked(&mut self, mouse: Mouse, fresh: bool) -> bool {
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
                let at = Position::new(mouse.column, mouse.row);
                self.read_press = Some((Press::Web, at, self.model.epoch()));
            }
            acted
        };
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
            draw::Hit::Stopped => return self.toggle_stopped() || held,
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
            draw::Hit::SettingsTab(_)
            | draw::Hit::StatsTab(_)
            | draw::Hit::SettingsClose
            | draw::Hit::PeekClose => {
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
        let count = draw::list_len(&self.fleet, self.stopped());
        let size = draw::dragged_to(drag, mouse, self.layout.area(), count);
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
    fn stretch(&mut self, mouse: Mouse) -> bool {
        (self.follow(mouse)).is_some_and(|(changed, first)| changed || first)
    }

    /// The button let go: a selection that never moved was a click; one that
    /// did asks for its copy, which the next frame paints and then ships.
    fn finish(&mut self, mouse: Mouse) -> bool {
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
    fn unselect(&mut self) -> bool {
        self.select.take().is_some_and(|held| held.sel.moved())
    }

    /// Drop a selection that the model has since moved out from under: a
    /// session, a tab or an overlay changed, even if it changed back. Whether
    /// a highlight went away.
    fn stale_selection(&mut self) -> bool {
        let epoch = self.model.epoch();
        self.read_press.take_if(|(_, _, pressed)| *pressed != epoch);
        let stale = self.select.take_if(|held| held.epoch != epoch);
        stale.is_some_and(|held| held.sel.moved())
    }

    /// A drag, or the copy it asked for, owns the screen: the cells it lit are
    /// the cells that ship. Not past [`LANE_HOLD`] without mouse input.
    fn holds_lane(&self) -> bool {
        (self.select.as_ref()).is_some_and(|held| {
            (held.down || held.stage != Stage::None) && held.touched.elapsed() < LANE_HOLD
        })
    }

    /// Keep a read back while a drag holds the screen: one per kind and name,
    /// the newest, carrying what the one it replaced alone had read.
    fn park(&mut self, mut answer: Answer) {
        let same = |old: &Answer| match (old, &answer) {
            (Answer::Fleet(_), Answer::Fleet(_)) => true,
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
    fn unpark(&mut self) -> bool {
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
    fn holds_peek(&self) -> bool {
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
    fn settle_selection(&mut self, buf: &mut Buffer) {
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
    /// proof, then the one client that pressed the key handed to a clean view
    /// of the seat's pane. The line to tell, refused or opened.
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
        if self.model.peek_open() {
            let seat = self
                .peek
                .target
                .as_ref()
                .map_or("", |(_, seat)| seat.as_str());
            return draw::Composer::Viewing { seat };
        }
        if self.model.reader_open() {
            return draw::Composer::Reading {
                path: &self.reader.path,
            };
        }
        match (self.model.selected(), &self.held) {
            (None, _) => draw::Composer::NoHome,
            (Some(_), Some(why)) => draw::Composer::Held { why },
            (Some(_), None) if !self.read_only.is_empty() && !self.writes_here() => {
                draw::Composer::ReadOnly {
                    why: &self.read_line,
                }
            }
            (Some(name), None) => draw::Composer::Home {
                home: name,
                view: self.writes_here().then_some(&self.draft_view),
                draft: &self.draft,
            },
        }
    }

    /// The columns this frame stands in, and the selection's draft wrapped
    /// to the width its column leaves.
    fn frame_columns(&mut self, area: ratatui_core::layout::Rect) {
        let columns = self.columns();
        self.model.set_columns(columns);
        if self.model.selected().is_some() {
            let size = Size {
                width: draw::column_width(area, self.model.split(), columns),
                height: draw::composer_pane(area),
            };
            let shown = self.shown_input().map(|i| (i.bare_view(size), i.draft()));
            (self.draft_view, self.draft) = shown.unwrap_or_default();
        }
    }

    /// One frame into `buf`, then the chat scroll bounded by what it held.
    /// Before the fleet is read, the selection's header stands on what its
    /// name alone says.
    fn frame(&mut self, buf: &mut Buffer) {
        if let Some(order) = self.pending.take() {
            self.fleet = std::mem::take(&mut self.fleet).arranged(&order);
        }
        self.frame_columns(buf.area);
        // An ask the lane shows needs no word of its own.
        let shown = |item: &Item, id: &str| matches!(&item.kind, Kind::Asked { id: ask, .. } | Kind::NotDelivered { id: ask, .. } if ask == id);
        let lane = Rc::clone(&self.lane);
        self.settling
            .take_if(|settling| lane.items.iter().any(|item| shown(item, &settling.id)));
        self.model.set_note(self.note());
        let selected = self.model.selected();
        let rows = self.fleet.rows.iter().map(|row| {
            let skip = row.home || selected == Some(row.name.as_str());
            let at = self.world.sessions.iter().find(|e| e.name == row.name);
            (row.name.as_str(), at.and_then(|e| e.answer_epoch), skip)
        });
        let unread = self.cue.frame(rows, Timestamp::now().epoch());
        self.model.set_unread(unread);
        self.model.set_stopped(self.stopped());
        let keys = self.keys();
        let (folds, open, sent) = self.fold_state();
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
        let dest = name.and_then(|name| self.destination(name));
        let wait = draw::Wait {
            fleet: !self.fleeted,
            lane: self.loading,
            keys: &keys,
            folds: &folds,
            open: &open,
            sent: &sent,
            dest: dest.as_ref().map(draw::Dest::of),
            md: Some(&self.md),
        };
        let mut drawn = draw::draw_with_layout(&screen, wait, buf);
        let cards = drawn
            .sessions()
            .map(|name| (name.to_owned(), self.card(name)));
        drawn.cards = cards.collect();
        let replaced = std::mem::replace(&mut self.layout, drawn);
        self.history.push((Instant::now(), replaced));
        if self.history.len() > HISTORY {
            self.lost = Some(self.history.remove(0).0);
        }
        self.model
            .settle(self.layout.list_start, self.layout.body_top);
        self.frame_panel(buf);
        // Held keys replay from the scroll they were read at, as one read. A
        // handed-over scroll waits for the lane it scrolls: until that is
        // read there is no bound to hold it to.
        let waits = self.restored && self.loading;
        self.restored = waits;
        if self.deferred.is_empty() && !waits {
            self.model
                .clamp_scroll(self.layout.max_scroll, self.layout.page_rows);
        }
        self.settings_due();
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
        self.paint_in_place(look.as_ref(), buf);
        let drawn = self.model.selected().map(|name| self.freeze(name));
        self.drawn = drawn;
        self.paint_modal(look.as_ref(), buf);
        self.settle_selection(buf);
    }

    /// Paint whichever panel stands in the chat's place: the peek or the
    /// reader.
    fn paint_in_place(&mut self, look: Option<&theme::Look>, buf: &mut Buffer) {
        if self.model.peek_open() {
            self.paint_peek(look, buf);
        }
        if self.model.reader_open() {
            self.paint_reader(look, buf);
        }
    }

    /// Paint the viewed seat's screen into the frame's chat column: its
    /// newest body with its age as of now, then keep the viewport inside
    /// what was drawn. The sidebar stands as drawn.
    fn paint_peek(&mut self, look: Option<&theme::Look>, buf: &mut Buffer) {
        self.peek_sync();
        self.peek_adopt();
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
        let bounds = draw::paint_peek_column(&view, look, buf, &mut self.layout);
        self.peek.painted = Some(age);
        self.model.set_peek_page_rows(bounds.visible);
        if self.deferred.is_empty() {
            self.model.clamp_peek(bounds.max_back);
        }
    }

    /// Paint the reader's file into the frame's chat column: wrapped to the
    /// width the panel leaves it, jumped to the line the link named once the
    /// body is first wrapped, then keep the viewport inside what was drawn.
    /// The sidebar stands as drawn.
    fn paint_reader(&mut self, look: Option<&theme::Look>, buf: &mut Buffer) {
        let lane = self.layout.lane_area().unwrap_or_default();
        let width = usize::from(lane.width);
        let visible = usize::from(lane.height.saturating_sub(1));
        self.reader
            .reflow(width, self.model.reader_top().unwrap_or(0));
        if self.reader.jump
            && let Some((_, flow)) = &self.reader.flow
        {
            self.reader.jump = false;
            if let Some(line) = self.reader.line {
                let (_, max_top) = reader::window(flow.rows(), visible, 0);
                let top = reader::top_for(flow.row_of(line), visible, max_top);
                self.model.set_reader_top(top);
            }
        }
        if let Some(line) = self.reader.anchor.take() {
            let top = match self.reader.shown() {
                Some(markdown::Shown::Rendered(r)) => r.row_of(line.saturating_add(1)),
                Some(markdown::Shown::Raw(f)) => f.row_of(line.saturating_add(1)),
                None => 0,
            };
            self.model.set_reader_top(top);
        }
        let view = reader::View {
            path: &self.reader.path,
            line: self.reader.line,
            body: self.reader.body.as_ref(),
            flow: self.reader.flow.as_ref().map(|(_, flow)| flow),
            top: self.model.reader_top().unwrap_or(0),
            links: self.reader.links,
        };
        let bounds = if let (Some(reader::Body::Text { lines, .. }), Some(shown)) =
            (&self.reader.body, self.reader.shown())
        {
            let markdown = markdown::View {
                path: view.path,
                line: view.line,
                lines,
                shown,
                refusal: self.reader.rendered.as_ref().and_then(|(_, r)| r.refusal()),
                top: view.top,
                links: view.links,
            };
            draw::paint_markdown_column(self.reader.index, &markdown, look, buf, &mut self.layout)
        } else {
            draw::paint_reader_column(self.reader.index, &view, look, buf, &mut self.layout)
        };
        self.model.set_reader_page_rows(bounds.visible);
        if self.deferred.is_empty() {
            self.model.clamp_reader(bounds.max_back);
        }
    }

    /// The card `name` shows as the app reads it now: what a frame freezes.
    fn card(&self, name: &str) -> menu::Card {
        menu::Card::new(
            name,
            self.ids.get(name).map_or("", String::as_str),
            self.sole.get(name).map(String::as_str),
            self.entry(name).map(|entry| entry.status),
            self.facts.get(name),
        )
    }

    /// The layout on screen when input read at `origin` was made.
    fn layout_at(&self, origin: Instant) -> &draw::Layout {
        (self.history.iter())
            .find(|(replaced, _)| origin <= *replaced)
            .map_or(&self.layout, |(_, layout)| layout)
    }

    /// Spend every key read at or before `origin` on the modal it changed.
    fn spend(&mut self, origin: Instant) {
        self.cut = self.cut.max(Some(origin));
    }

    /// A key read at or before the last modal change: spent.
    fn spent(&self, origin: Instant) -> bool {
        self.cut.is_some_and(|cut| origin <= cut)
    }

    /// The menu on `card`, drawn by the frame input read at `origin` was made
    /// on, or the hint row says none opened; that read is spent either way.
    /// Opening selects nothing.
    fn open_menu(&mut self, card: Option<menu::Card>, origin: Instant) -> bool {
        self.spend(origin);
        let Some(card) = card else {
            let line = "menu not opened: no card is on screen for it".to_owned();
            self.flash = Some((line, Instant::now()));
            return true;
        };
        if card.items().is_empty() {
            let line = format!(
                "menu not opened: {} has nothing to start or stop",
                card.name
            );
            self.flash = Some((line, Instant::now()));
            return true;
        }
        self.unselect();
        self.modal = Some(menu::Modal::Menu(menu::Menu::new(card)));
        true
    }

    /// `s` on the selection: the menu's one item on the card the frame drew —
    /// Start a stopped session, or ask the Stop question of a running one.
    fn toggle(&mut self, origin: Instant) -> bool {
        let card = (self.model.selected()).and_then(|name| self.layout.cards.get(name));
        let opened = self.open_menu(card.cloned(), origin);
        if let Some(menu::Modal::Menu(menu)) = &self.modal
            && let Some(&item) = menu.card.items().first()
        {
            self.act(item);
        }
        opened
    }

    /// A right press: the menu on the card the frame on screen at `origin`
    /// drew under it. One made on a frame the bound dropped opens none.
    fn menu_press(&mut self, mouse: Mouse, origin: Instant) -> bool {
        let ended = self.model.let_go();
        if self.lost.is_some_and(|lost| origin <= lost) {
            self.spend(origin);
            let line = "menu not opened: its frame is gone".to_owned();
            self.flash = Some((line, Instant::now()));
            return true;
        }
        let layout = self.layout_at(origin);
        let Some(draw::Hit::Session(name)) = layout.hit(mouse) else {
            return ended;
        };
        let card = layout.cards.get(&name).cloned();
        self.open_menu(card, origin)
    }

    /// Input the open modal takes only once a frame showing it was written
    /// before the input was read: earlier input is dropped, and the hint row
    /// says to press again.
    fn unseen(&mut self, origin: Instant) -> bool {
        let Some(modal) = &self.modal else {
            return false;
        };
        if modal.shown_at().is_some_and(|shown| origin > shown) {
            return false;
        }
        let word = modal.word();
        let line = format!("not taken: the {word} was not on screen yet - press again");
        self.flash = Some((line, Instant::now()));
        true
    }

    /// A key with a modal open: nothing under the modal sees it.
    fn modal_key(&mut self, key: &Key, origin: Instant) -> bool {
        if self.unseen(origin) {
            return true;
        }
        let Some(modal) = self.modal.as_mut() else {
            return false;
        };
        let step = modal.key(key);
        self.stepped(step, origin)
    }

    /// A mouse report with a modal open: motion and the wheel die, a press
    /// acts on the frame it was made on.
    fn modal_mouse(&mut self, mouse: Mouse, origin: Instant) -> bool {
        let right = mouse.kind == MouseKind::Menu;
        if !right && mouse.kind != MouseKind::Click {
            return false;
        }
        if self.unseen(origin) {
            return true;
        }
        if self.lost.is_some_and(|lost| origin <= lost) {
            let line = "click not taken: its frame is gone".to_owned();
            self.flash = Some((line, Instant::now()));
            return true;
        }
        let hit = self.layout_at(origin).hit(mouse);
        let Some(modal) = &self.modal else {
            return false;
        };
        let step = modal.press(right, hit.as_ref());
        self.stepped(step, origin)
    }

    /// Carry out what input read at `origin` did to the modal: one that ends
    /// it spends that read.
    fn stepped(&mut self, step: menu::Step, origin: Instant) -> bool {
        if step.ends() {
            self.spend(origin);
        }
        match step {
            menu::Step::Kept => return false,
            menu::Step::Moved => {}
            menu::Step::Close => self.close_modal(),
            menu::Step::Act(item) => self.act(item),
            menu::Step::Affirm => self.affirm(),
        }
        true
    }

    /// Take the open modal down. A menu still waiting for its dialog's rows
    /// tells the reader to drop the read.
    fn close_modal(&mut self) {
        let waiting =
            matches!(&self.modal, Some(menu::Modal::Menu(menu)) if menu.reading.is_some());
        self.modal = None;
        if waiting && let Some(ask) = &self.ask {
            let _ = ask.send(Request::Records(None));
        }
    }

    /// `item` of the open menu: a dialog is asked of the reader; else its
    /// card's refusal, else Start or the Stop or End question. The menu
    /// closes either way, but for a dialog, which it waits on.
    fn act(&mut self, item: menu::Item) {
        if let Some(dialog) = item.dialog() {
            return self.ask_records(item, dialog);
        }
        let Some(menu::Modal::Menu(menu)) = self.modal.take() else {
            return;
        };
        let (name, card) = (menu.card.name.clone(), menu.card);
        if let Some(why) = card.refusal() {
            let line = format!("refused: {} {name}: {why}; nothing done", item.word());
            return self.tell(&name, line);
        }
        match item {
            menu::Item::Start => {
                let line = (self.start(&card))
                    .unwrap_or_else(|why| format!("refused: Start {name}: {why}; nothing started"));
                self.tell(&name, line);
            }
            menu::Item::Stop => match self.question(card) {
                Ok(question) => self.modal = Some(menu::Modal::Question(question)),
                Err(line) => self.tell(&name, line),
            },
            menu::Item::End => match self.end_question(card) {
                Ok(question) => self.modal = Some(menu::Modal::End(question)),
                Err(why) => {
                    let line = format!("refused: End {name}: {why}; nothing ended");
                    self.tell(&name, line);
                }
            },
            menu::Item::Activity | menu::Item::Memos => {}
        }
    }

    /// The menu's Activity… or Memos…: the reader reads the rows, under the
    /// identity the card drew; the menu stays until they land. A read-only
    /// item takes the card's refusals as they are: the fold names its own
    /// gaps, never fills them. A record directory ae cannot read is told.
    fn ask_records(&mut self, item: menu::Item, dialog: menu::Dialog) {
        let Some(menu::Modal::Menu(menu)) = &mut self.modal else {
            return;
        };
        if menu.reading.is_some() {
            return;
        }
        let card = menu.card.clone();
        let Some(dir) = self.dirs.get(&card.name).cloned() else {
            self.modal = None;
            let word = item.word();
            let why = "ae read no record directory for it";
            return self.tell(&card.name, format!("refused: {word} {}: {why}", card.name));
        };
        let generation = self.records_generation.wrapping_add(1);
        self.records_generation = generation;
        menu.reading = Some(menu::Asked { generation, dialog });
        let line = format!("reading {} of {}…", item.word(), card.name);
        self.flash = Some((line, Instant::now()));
        let ask = Box::new(RecordsAsk {
            generation,
            dialog,
            session: card.name,
            uuid: card.uuid,
            dir,
            live: !card.stopped,
        });
        if let Some(tx) = &self.ask {
            let _ = tx.send(Request::Records(Some(ask)));
        }
    }

    /// The rows a dialog was asked for. Only the menu that asked takes them:
    /// an answer for a closed menu, or for an older ask, is dropped.
    fn records_landed(&mut self, generation: u64, rows: Vec<CellRow>) {
        let Some(menu::Modal::Menu(menu)) = &self.modal else {
            return;
        };
        let Some(asked) = menu.reading.filter(|asked| asked.generation == generation) else {
            return;
        };
        let records = menu::Records::new(asked.dialog, &menu.card.name, rows);
        self.modal = Some(menu::Modal::Records(records));
        if (self.flash.as_ref()).is_some_and(|(line, _)| line.starts_with("reading ")) {
            self.flash = None;
        }
    }

    /// An outcome, on the hint row now and in `name`'s lane.
    fn tell(&mut self, name: &str, line: String) {
        self.flash = Some((line.clone(), Instant::now()));
        self.notice(name, line);
        self.show();
    }

    /// What a Start or the Stop question freezes about the asking client and
    /// its server: the record must still name the card's incarnation, tmux must
    /// be reachable, the session recorded on this very server, and one client
    /// view this app.
    fn asker(&self, card: &menu::Card) -> Result<Asker, String> {
        let name = &card.name;
        let dir = (self.dirs.get(name)).ok_or("ae read no record directory for it")?;
        if loader::sole_uuid(dir).as_deref() != Some(card.uuid.as_str()) {
            return Err(REPLACED.to_owned());
        }
        let (Some(server), Some(pane)) = (&self.server, &self.pane) else {
            return Err(Refusal::NoTmux.why());
        };
        let mut sockets = crate::fleet_read::SocketPaths::live();
        let recorded = crate::session_launch::recorded_server_resolved(dir);
        if !recorded.is_some_and(|recorded| sockets.proven_same(server, &recorded)) {
            return Err(Refusal::Elsewhere(name.clone()).why());
        }
        let clients = transport::observe_clients(server)
            .ok_or_else(|| Refusal::Unread("the tmux clients".to_owned()).why())?;
        let client = open::copy_client(&clients, pane).map_err(|why| why.why())?;
        let menu = transport::observe_menu_client(server, &client)
            .ok_or("the asking client did not answer")?;
        let identity =
            transport::observe_server_identity(server).ok_or("the tmux server did not answer")?;
        Ok(Asker {
            server: server.clone(),
            client,
            client_pid: menu.pid,
            session: menu.session_name,
            server_pid: identity.pid,
            server_start: identity.start,
        })
    }

    /// The Stop question for `card`: every fact its answer is proven against
    /// is read now and frozen in it, or the refusal line for the first that
    /// does not hold.
    fn question(&self, mut card: menu::Card) -> Result<menu::Question, String> {
        let name = card.name.clone();
        let refuse = |why: &str| format!("refused: Stop {name}: {why}; nothing stopped");
        if !self.attended() {
            return self.unattended_question(card).map_err(|why| refuse(&why));
        }
        let asker = self.asker(&card).map_err(|why| refuse(&why))?;
        let id = crate::lifecycle::live_id(&asker.server, &name)
            .ok_or_else(|| refuse("tmux does not list it on this server"))?;
        if card.id.as_ref().is_some_and(|painted| *painted != id) {
            return Err(refuse(
                "its tmux session was replaced since the menu was drawn",
            ));
        }
        card.home = asker.session == name;
        let expect = crate::lifecycle::StopExpectation {
            session_id: id,
            uuid: card.uuid.clone(),
            server_pid: asker.server_pid,
            server_start: asker.server_start,
            deadline: Timestamp::now().epoch() + crate::session_menu::CONFIRM_WINDOW_SECS,
            client: asker.client,
            client_pid: asker.client_pid,
            server: asker.server,
            require_meta_agent: false,
        };
        Ok(menu::Question::new(card, expect))
    }

    /// Whether this app runs inside a tmux client: a Start or Stop then rides
    /// that client's proofs. Outside tmux there is none to name.
    fn attended(&self) -> bool {
        self.server.is_some() && self.pane.is_some()
    }

    /// The tmux server `name`'s record names, proven to be the one the card
    /// was drawn from: its record directory still holds the card's
    /// incarnation, and the server it records is a positive selector.
    fn recorded(&self, card_name: &str, uuid: &str) -> Result<(PathBuf, ServerId), String> {
        let dir = (self.dirs.get(card_name)).ok_or("ae read no record directory for it")?;
        if loader::sole_uuid(dir).as_deref() != Some(uuid) {
            return Err(REPLACED.to_owned());
        }
        match crate::session_launch::recorded_server_resolved(dir) {
            Some(server @ ServerId::Selected(_)) => Ok((dir.clone(), server)),
            _ => Err("its record names no tmux server".to_owned()),
        }
    }

    /// The Stop question outside tmux: nobody is attached to answer, so the
    /// app's own modal is the confirmation, and what tmux and the record say
    /// of the session is frozen in it from the server the record names — its
    /// process, its start, the session id — for the supervisor to prove again
    /// under the lock.
    fn unattended_question(&self, mut card: menu::Card) -> Result<menu::Question, String> {
        let name = card.name.clone();
        let (_, server) = self.recorded(&name, &card.uuid)?;
        let identity = transport::observe_server_identity(&server)
            .ok_or("the tmux server its record names did not answer")?;
        let id = crate::lifecycle::live_id(&server, &name)
            .ok_or("tmux does not list it on the server its record names")?;
        if card.id.as_ref().is_some_and(|painted| *painted != id) {
            return Err("its tmux session was replaced since the menu was drawn".to_owned());
        }
        card.home = false;
        let deadline = Timestamp::now().epoch() + crate::session_menu::CONFIRM_WINDOW_SECS;
        let expect = crate::lifecycle::StopExpectation::unattended(
            id,
            card.uuid.clone(),
            identity,
            deadline,
            server,
        );
        Ok(menu::Question::new(card, expect))
    }

    /// The End question: the modal is the confirmation, inside tmux or not,
    /// so what tmux and the record say of the session is frozen in it from
    /// the server the record names, for the owner to prove under the lock.
    fn end_question(&self, mut card: menu::Card) -> Result<menu::EndQuestion, String> {
        let root = (self.root.as_ref()).ok_or("ae app has no state root to end under")?;
        let (_, server) = self.recorded(&card.name, &card.uuid)?;
        let deadline = Timestamp::now().epoch() + crate::session_menu::CONFIRM_WINDOW_SECS;
        let expect = crate::lifecycle::end::EndExpectation::unattended(
            root,
            &card.name,
            &card.uuid,
            server.clone(),
            card.stopped,
            deadline,
        )?;
        if let Some(live) = expect.live_id() {
            if card.stopped {
                return Err("it was started since the menu was drawn".to_owned());
            }
            if card.id.as_ref().is_some_and(|painted| painted != live) {
                return Err("its tmux session was replaced since the menu was drawn".to_owned());
            }
            card.home = self.runs_in(&server, live);
        }
        Ok(menu::EndQuestion::new(card, expect))
    }

    /// Whether this app's own pane is in the tmux session `id` on `server`.
    fn runs_in(&self, server: &ServerId, id: &str) -> bool {
        let (Some(own), Some(pane)) = (&self.server, &self.pane) else {
            return false;
        };
        crate::fleet_read::SocketPaths::live().proven_same(own, server)
            && transport::observe_pane_session_identity(own, pane)
                .is_some_and(|session| session.id == id)
    }

    /// The End question answered End: its deadline and the record proven once
    /// more against what it froze, never captured again, then the owner,
    /// detached. The journal's length is taken first: the reader believes only
    /// what is written past it, and only of the incarnation that was asked.
    fn end(&self, question: &menu::EndQuestion) -> Result<String, String> {
        let (expect, name) = (&question.expect, &question.card.name);
        if Timestamp::now().epoch() > expect.deadline() {
            return Err("the question expired - ask again".to_owned());
        }
        let root = (self.root.as_ref()).ok_or("ae app has no state root to end under")?;
        let (dir, server) = self.recorded(name, expect.uuid())?;
        if server != *expect.server() {
            return Err("it is recorded on a different tmux server than the one the question was asked about".to_owned());
        }
        let position = match crate::store::open(&dir).events_source() {
            crate::store::SourceRead::Absent => 0,
            crate::store::SourceRead::Ready(bytes) => bytes.len(),
            crate::store::SourceRead::Invalid(why) | crate::store::SourceRead::Unreadable(why) => {
                return Err(format!("ae cannot read its journal: {why}"));
            }
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let detach = |argv: &crate::lifecycle::DetachedArgv| (self.detach)(argv);
        let code = crate::lifecycle::end::end_confirmed_via(
            root, expect, name, detach, &mut out, &mut err,
        )
        .map_err(|why| why.to_string())?;
        out.extend(err);
        let line = String::from_utf8_lossy(&out).trim().to_owned();
        if code != 0 {
            return Err(line);
        }
        let ask = Box::new(EndingAsk {
            name: name.clone(),
            dir,
            uuid: expect.uuid().to_owned(),
            position,
            asked: Instant::now(),
        });
        if let Some(tx) = &self.ask {
            let _ = tx.send(Request::Ending(ask));
        }
        Ok(line)
    }

    /// Start `card`'s stopped session outside tmux: the session resumed by name
    /// with nothing attached, detached from this app, once its record still
    /// names the incarnation the card was drawn from. No client moves.
    fn start_unattended(&self, card: &menu::Card) -> Result<String, String> {
        let name = &card.name;
        let dir = (self.dirs.get(name)).ok_or("ae read no record directory for it")?;
        if loader::sole_uuid(dir).as_deref() != Some(card.uuid.as_str()) {
            return Err(REPLACED.to_owned());
        }
        let named = crate::session_launch::name::is_session_name(name)
            && !crate::entry::name_is_routed_verb(name);
        if !named {
            return Err("ae cannot resume a session of that name from here".to_owned());
        }
        let argv =
            crate::lifecycle::resume_argv(name).ok_or("ae cannot name its own executable")?;
        if !(self.detach)(&argv) {
            return Err("ae could not start the resume job".to_owned());
        }
        Ok(format!("Starting '{name}'."))
    }

    /// Start `card`'s stopped session: the picker row's own resume job — the
    /// ordinary launch owner behind its captured-client proofs — run in the
    /// background by tmux. Its last step moves this client to the session.
    fn start(&self, card: &menu::Card) -> Result<String, String> {
        if !self.attended() {
            return self.start_unattended(card);
        }
        let asker = self.asker(card)?;
        let launcher =
            crate::picker_launcher_for(&asker.server).ok_or("ae cannot name its own executable")?;
        let resume = crate::orchestrator::PickerResume {
            client: &asker.client,
            client_pid: &asker.client_pid,
            server_pid: &asker.server_pid,
            server_start: &asker.server_start,
            deadline: Timestamp::now().epoch() + crate::session_menu::CONFIRM_WINDOW_SECS,
            launcher: &launcher,
        };
        let argv = crate::orchestrator::resume_argv(&card.name, &resume);
        if !transport::run_shell_background(&asker.server, &argv) {
            return Err("tmux refused the start job".to_owned());
        }
        Ok(format!("Starting '{}'.", card.name))
    }

    /// A question answered yes. Stop: its deadline, the record and the asking
    /// client proven once more against what it froze, never captured again,
    /// then the one guarded Stop owner, detached. End: [`App::end`].
    fn affirm(&mut self) {
        match self.modal.take() {
            Some(menu::Modal::Question(question)) => {
                let name = question.card.name.clone();
                let line = (self.stop(&question))
                    .unwrap_or_else(|why| format!("refused: Stop {name}: {why}; nothing stopped"));
                self.tell(&name, line);
            }
            Some(menu::Modal::End(question)) => {
                let name = question.card.name.clone();
                let line = (self.end(&question))
                    .unwrap_or_else(|why| format!("refused: End {name}: {why}; nothing ended"));
                self.tell(&name, line);
            }
            _ => {}
        }
    }

    /// [`App::affirm`]'s proofs, then the owner's own line.
    fn stop(&self, question: &menu::Question) -> Result<String, String> {
        let (expect, name) = (&question.expect, &question.card.name);
        if Timestamp::now().epoch() > expect.deadline {
            return Err("the question expired - ask again".to_owned());
        }
        let dir = (self.dirs.get(name)).ok_or("ae read no record directory for it")?;
        if loader::sole_uuid(dir).as_deref() != Some(expect.uuid.as_str()) {
            return Err(REPLACED.to_owned());
        }
        if !expect.is_attended() {
            return self.stop_unattended(question);
        }
        let (Some(pane), Some(root)) = (&self.pane, &self.root) else {
            return Err(Refusal::NoTmux.why());
        };
        let clients =
            transport::observe_clients(&expect.server).ok_or("the tmux clients did not answer")?;
        let client = open::copy_client(&clients, pane).map_err(|why| why.why())?;
        let pid = transport::observe_menu_client(&expect.server, &client).map(|asker| asker.pid);
        if client != expect.client || pid.as_ref() != Some(&expect.client_pid) {
            return Err("the client that was asked changed or is gone".to_owned());
        }
        let (mut out, mut err) = (Vec::new(), Vec::new());
        crate::lifecycle::stop_confirmed(root, expect, name, &mut out, &mut err)
            .map_err(|why| why.to_string())?;
        out.extend(err);
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// [`App::stop`] for a question asked outside tmux: the record must still
    /// name the incarnation and the very server the question froze, then the
    /// supervisor is handed the frozen identity to prove under the lock.
    fn stop_unattended(&self, question: &menu::Question) -> Result<String, String> {
        let (expect, name) = (&question.expect, &question.card.name);
        let root = self
            .root
            .as_ref()
            .ok_or("ae app has no state root to stop under")?;
        let (_, server) = self.recorded(name, &expect.uuid)?;
        if server != expect.server {
            return Err("it is recorded on a different tmux server than the one the question was asked about".to_owned());
        }
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let detach = |argv: &crate::lifecycle::DetachedArgv| (self.detach)(argv);
        crate::lifecycle::stop_confirmed_via(root, expect, name, detach, &mut out, &mut err)
            .map_err(|why| why.to_string())?;
        out.extend(err);
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// Paint the open modal over the frame. One that does not fit whole is
    /// taken down, the hint row says what it needed, and every key read so
    /// far is spent: nothing it held can act.
    fn paint_modal(&mut self, look: Option<&theme::Look>, buf: &mut Buffer) {
        let Some(modal) = &self.modal else {
            return;
        };
        let icons = look.is_none_or(|look| look.icons);
        match menu::paint(modal, draw::Paint::of(look), icons, buf, &mut self.layout) {
            Ok(()) => self.layout.cursor = None,
            Err(why) => {
                self.modal = None;
                self.spend(Instant::now());
                self.flash = Some((why, Instant::now()));
                self.again = true;
            }
        }
    }

    /// The frame just written showed the open modal whole at `at`: the first
    /// such frame dates which input counts as made on it.
    fn frame_shown(&mut self, at: Instant) {
        if self.layout.shows_modal()
            && let Some(modal) = self.modal.as_mut()
        {
            modal.stamp(at);
        }
    }
}

/// Why a menu action refuses an incarnation its record no longer names.
/// What a Start or the Stop question freezes of the asking client and server.
struct Asker {
    server: ServerId,
    client: String,
    client_pid: String,
    /// The session the asking client is in.
    session: String,
    server_pid: String,
    server_start: String,
}

const REPLACED: &str =
    "its record no longer names the session_id the menu was drawn from - replaced or changed";

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

/// One frame, drawn exactly as `Terminal::draw` draws it — autoresize, the
/// callback into the current buffer, the buffer diff, the cursor shown or
/// hidden, the buffers swapped, the backend flushed, in that order — except
/// that `paint` also answers the links the frame carries. The backend takes
/// them before the diff, and every cell it must write again — those a link
/// covers now and those one covered last frame — is marked so the diff cannot
/// skip it: a cell's symbol and style say nothing of the link the terminal
/// holds on it. With no runs and none before, no cell is marked and the bytes
/// are `Terminal::draw`'s.
fn draw_frame<W: Write>(
    terminal: &mut Terminal<backend::Ansi<W>>,
    paint: impl FnOnce(&mut Buffer) -> (Option<Position>, Vec<backend::Run>),
) -> std::io::Result<()> {
    terminal.autoresize()?;
    let (cursor, runs) = paint(terminal.current_buffer_mut());
    let forced = terminal.backend_mut().links(runs);
    let buffer = terminal.current_buffer_mut();
    for at in forced {
        if let Some(cell) = buffer.cell_mut(at) {
            cell.set_diff_option(CellDiffOption::AlwaysUpdate);
        }
    }
    terminal.apply_buffer_with_cursor(cursor)?;
    Ok(())
}

/// `ae app [session]`.
///
/// # Errors
///
/// [`crate::error::Error::Io`] when the terminal or `err` cannot be written.
#[allow(
    clippy::too_many_lines,
    reason = "the one loop: its setup, then draw, show, drain"
)]
pub fn run(
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let Some(Operands {
        session,
        own,
        handed,
    }) = operands(tail)
    else {
        write!(err, "{USAGE}")?;
        return Ok(crate::entry::EXIT_USAGE);
    };
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        write!(err, "{NO_TERMINAL}")?;
        return Ok(crate::EXIT_UNAVAILABLE);
    }
    let Some(root) = crate::doors::state_root(crate::shape::current()) else {
        writeln!(err, "ae: {}", crate::shape::NO_STATE_ROOT)?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    // The human's operands as typed: what an exec restart hands on, unchanged.
    let typed: Vec<String> = session.iter().cloned().collect();
    let home = session.or_else(crate::calling_session_name);
    if let Some(name) = home.as_deref()
        && console::locate(&root, name).is_none()
    {
        let line = format!("ae app: no session named {name}");
        writeln!(err, "{}", crate::board::terminal_text(&line))?;
        return Ok(crate::EXIT_UNAVAILABLE);
    }
    let Some((reads, pause)) = console::term::stdin_reads_pausable() else {
        write!(err, "{NO_TERMINAL}")?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let (wake, wakes) = mpsc::channel();
    let (caller, pane) = (doors::caller_server(), doors::calling_pane_id());
    let hint = !(own || handed.is_some()) && {
        let at = caller.as_ref().zip(pane.as_deref());
        let stamp = at.and_then(|(server, pane)| transport::observe_pane_console(server, pane));
        says_hint(at.is_some(), stamp.as_deref())
    };
    let (keep, landed) = handed_on(handed);
    let own = (caller.clone(), pane.clone());
    let reader = reader_of(&root, home.clone(), own, landed);
    let notices = wake.clone();
    let started = forward(reads, wake.clone()).and_then(|()| loader::spawn(reader, wake));
    let (ask, reader) = match started {
        Ok(started) => started,
        Err(why) => {
            writeln!(err, "ae app: {why}")?;
            return Ok(crate::EXIT_UNAVAILABLE);
        }
    };
    // The store's upkeep: an app that cannot start it runs without it. Every
    // return below drops it, which stops the thread.
    let mut upkeep = maintainer::Upkeep::start(root.clone(), notices);
    if hint {
        // Best effort: a stderr that cannot be written must not stop an app that opened before.
        let _ = writeln!(err, "{HINT}");
    }
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
    app.root = Some(root);
    app.keep = keep;
    let mut keys = Keys::app();
    let (mut dirty, mut first, mut restart) = (true, None, None);
    let code = loop {
        if let Some(now) = tty.size().filter(|now| *now != size) {
            size = now;
            terminal.backend_mut().resize(size);
            dirty = true;
        }
        if dirty || !app.queued.is_empty() {
            draw_frame(&mut terminal, |buf| {
                app.frame(buf);
                (app.layout.cursor, app.layout.links.clone())
            })?;
            app.frame_shown(Instant::now());
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
        if let Some(ask) = app.pane_view.take() {
            app.deferred.clear();
            let line = paneview::run(&tty, &pause, &ask);
            app.flash = Some((line, Instant::now()));
            app.resumed_at = Some(Instant::now());
            // A key half decoded before the hand-over is the pane's, not ours.
            keys = Keys::app();
            terminal.clear()?;
            dirty = true;
        }
        let mut outside = Real {
            tty: &tty,
            terminal: &mut terminal,
        };
        match step(&mut app, &mut outside, &mut upkeep, &typed, err)? {
            Step::Stay(redraw) => dirty |= redraw,
            Step::Respawn(version, respawn) => {
                restart = Some((version, respawn));
                break 0;
            }
            Step::Quit(code) => return Ok(code),
        }
    };
    end(tty, app, upkeep, code, restart, err)
}

/// The reader of an app for `home`, in the `own` server and pane tmux names,
/// told the version an exec `landed` this app in when that is not this one.
fn reader_of(
    root: &Path,
    home: Option<String>,
    own: (Option<ServerId>, Option<String>),
    landed: Option<String>,
) -> loader::Reader {
    let declared = doors::declared_server(crate::shape::current());
    let server = doors::launch_target(declared.as_ref());
    let mut reader = loader::Reader::new(root.to_path_buf(), home, server);
    let global = doors::config_file(crate::shape::current(), root);
    reader.settings_paths(loader::SettingsPaths {
        home: doors::home(),
        local: doors::local_config(&doors::cwd(), Some(&global)),
        global,
    });
    reader.own_pane(own.0, own.1).landed(landed);
    reader
}

/// What the view an exec restart handed on behind [`handover::FLAG`] gives
/// this process: the view it keeps, or the version it was meant to be.
fn handed_on(text: Option<String>) -> (Option<Keep>, Option<String>) {
    let now = Timestamp::now().epoch();
    match text.map(|text| upgrade::carried(&text, std::process::id(), crate::shape::VERSION, now)) {
        Some(upgrade::Carried::View(view, at)) => (Some(Keep { view, at }), None),
        Some(upgrade::Carried::Landed(version)) => (None, Some(version)),
        Some(upgrade::Carried::Ignored) | None => (None, None),
    }
}

/// What the loop does about the restart due now.
enum Step {
    /// Nothing due, or an exec that failed and took the terminal back: go on,
    /// redrawing everything when it did.
    Stay(bool),
    /// End into the chat window's respawn.
    Respawn(String, toggle::Restart),
    /// The terminal was not taken back: end with this code.
    Quit(u8),
}

/// The restart due now, if any: a chat window ends into its respawn; any other
/// app replaces its process with the core the link names, handing on the
/// human's `typed` operands and the view, the terminal put back first exactly
/// as on quit — exec keeps the pid, so the shell that started it stays. An
/// exec returns only when it failed: the SAME terminal is taken back and the
/// app told why, so it never tries that version again.
fn step(
    app: &mut App,
    outside: &mut dyn Outside,
    upkeep: &mut maintainer::Upkeep,
    typed: &[String],
    err: &mut impl Write,
) -> crate::error::Result<Step> {
    // A thread a failed exec owes starts here, on the first turn after the
    // stopped one ended.
    upkeep.tend();
    // `r` on the Statistics tab goes to the thread, or waits for one to run.
    let mut waits = false;
    if let Some(generation) = app.take_reset()
        && upkeep.reset(generation) == maintainer::Handed::Held
    {
        app.reset_waits(generation);
        waits = true;
    }
    let (version, link) = match app.restart_due(Instant::now()) {
        None => return Ok(Step::Stay(waits)),
        Some((version, upgrade::Via::Respawn(respawn))) => {
            return Ok(Step::Respawn(version, respawn));
        }
        Some((version, upgrade::Via::Exec { link })) => (version, link),
    };
    let pid = format!("%{}", std::process::id());
    let view = app.handover(&pid, &version, Timestamp::now().epoch());
    let view = view.map(|handover| handover.render());
    let argv = upgrade::exec_argv(&link, typed, view, &version, crate::shape::VERSION);
    // The store's upkeep stops before the process is replaced: an exec never
    // returns to stop it.
    upkeep.pause(maintainer::STOP_BOUND);
    outside.suspend();
    let why = outside.exec(&argv).to_string();
    if let Err(lost) = outside.resume() {
        let said = format!("ae app: could not restart into ae {version} - {why}; {lost}");
        writeln!(err, "{}", terminal_text(&said))?;
        return Ok(Step::Quit(crate::entry::EXIT_FAILED));
    }
    upkeep.resume();
    app.failed = Some((version, why));
    outside.clear()?;
    Ok(Step::Stay(true))
}

/// What a due restart touches outside the app: the terminal and the process.
trait Outside {
    /// Give the terminal back before an exec.
    fn suspend(&mut self);
    /// Replace this process with `argv`: returns only when that failed.
    fn exec(&mut self, argv: &[String]) -> std::io::Error;
    /// Take the terminal back after a failed exec.
    ///
    /// # Errors
    ///
    /// The terminal could not be taken back, and why.
    fn resume(&mut self) -> Result<(), String>;
    /// Clear the frame, so the next one is drawn whole.
    ///
    /// # Errors
    ///
    /// The terminal could not be written.
    fn clear(&mut self) -> std::io::Result<()>;
}

/// The app's own terminal, and the real exec.
struct Real<'a, W: Write> {
    tty: &'a tty::Tty,
    terminal: &'a mut Terminal<backend::Ansi<W>>,
}

impl<W: Write> Outside for Real<'_, W> {
    fn suspend(&mut self) {
        self.tty.suspend();
    }

    fn exec(&mut self, argv: &[String]) -> std::io::Error {
        crate::run::exec_unchanged(argv)
    }

    fn resume(&mut self) -> Result<(), String> {
        self.tty.resume()
    }

    fn clear(&mut self) -> std::io::Result<()> {
        self.terminal.clear()
    }
}

/// `ae app`'s operands.
struct Operands {
    /// The home session as typed.
    session: Option<String>,
    /// [`OWN`] was given: an app start ae made itself.
    own: bool,
    /// The view an exec restart handed on behind [`handover::FLAG`].
    handed: Option<String>,
}

/// `ae app`'s operands: the home session, then [`OWN`] once, then the view an
/// exec restart handed on behind [`handover::FLAG`], last. `None` is a usage
/// error.
fn operands(tail: &[String]) -> Option<Operands> {
    let (session, rest) = match tail.split_first() {
        Some((name, rest)) if !name.starts_with('-') => (Some(name.clone()), rest),
        _ => (None, tail),
    };
    let (own, rest) = match rest.split_first() {
        Some((flag, more)) if flag == OWN => (true, more),
        _ => (false, rest),
    };
    let handed = match rest {
        [] => None,
        [flag, text] if flag == handover::FLAG => Some(text.clone()),
        _ => return None,
    };
    Some(Operands {
        session,
        own,
        handed,
    })
}

/// Whether a start that no marker or carrier calls ae's own says [`HINT`],
/// given whether it runs in a tmux pane and that pane's `@ae_console` stamp,
/// `None` when tmux did not answer ([`upgrade::via`] reads it the same way). A
/// stamped pane is a chat window, ae's own start; a silent tmux is doubt, and
/// doubt says nothing.
fn says_hint(in_pane: bool, stamp: Option<&str>) -> bool {
    !in_pane || stamp == Some("")
}

/// How `ae app` ends, its terminal put back first: restarted into the core
/// the command link names when that was due, else with the line that says
/// how to start it again.
fn end(
    tty: tty::Tty,
    mut app: App,
    upkeep: maintainer::Upkeep,
    code: u8,
    restart: Option<(String, toggle::Restart)>,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    // The store's upkeep stops before anything else.
    upkeep.stop(maintainer::STOP_BOUND);
    drop(tty);
    app.finish_flight(err, FLIGHT_QUIT_BOUND)?;
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
    let mut held = std::mem::take(&mut app.deferred);
    // A key held from before a pane view was typed for the seat, not here.
    if let Some(resumed) = app.resumed_at {
        held.retain(|(_, origin)| paneview::keep_key(*origin, resumed));
    }
    let (mut redraw, mut fed) = (take_keys(app, held)?, false);
    while app.queued.is_empty()
        && app.pane_view.is_none()
        && let Some(wake) = first.take().or_else(|| wakes.try_recv().ok())
    {
        let keyed = match wake {
            Wake::Keys(stamp, bytes) => {
                if app
                    .resumed_at
                    .is_some_and(|resumed| !paneview::keep_key(stamp, resumed))
                {
                    continue;
                }
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
            Wake::Notice(line) => {
                app.flash = Some((line, Instant::now()));
                redraw = true;
                continue;
            }
            Wake::StatsReset {
                generation,
                outcome,
            } => {
                app.reset_landed(generation, outcome);
                redraw = true;
                continue;
            }
            Wake::Closed => return None,
        };
        redraw |= take_keys(app, keyed)?;
    }
    if !fed && app.pane_view.is_none() {
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
    if let Key::Left | Key::Right = key {
        // An arrow steps the Statistics tab's inner row and means nothing on
        // any other tab.
        return Some(app.model.arrows_step_stats() && app.stats_tab(*key == Key::Right));
    }
    for key in model::browse_keys(key) {
        // The overlay branch returns no Quit, Select or Compose; a close
        // shows as the state it leaves behind, told below.
        redraw |= settings_act(app, key);
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

/// Feed `keys` to the seat view; a close they cause tells the reader.
/// `y` copies no turn here: it only says how copying works in the view.
fn peek_keys(app: &mut App, keys: Vec<model::Key>) -> bool {
    let mut redraw = false;
    for key in keys {
        if key == model::Key::Copy {
            app.flash = Some(("drag to copy text".to_owned(), Instant::now()));
            redraw = true;
            continue;
        }
        redraw |= app.model.key(key, &app.fleet, false) == model::Act::Redraw;
    }
    if !app.model.peek_open() {
        app.close_peek();
        redraw = true;
    }
    redraw
}

/// Take one key with the reader open: `Some(redraw)`, or `None` to quit.
/// The reader owns every key: it scrolls, it closes, and a paste or any
/// other key dies. Bytes after a mid-chunk close resume the actual mode.
fn reader_key(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    if *key == Key::Interrupt {
        return None;
    }
    if matches!(key, Key::Text(_)) {
        return browse(app, key, origin);
    }
    Some(reader_keys(app, model::browse_keys(key)))
}

/// Feed `keys` to the reader; a close they cause tells the reader thread.
/// `y` copies no turn here: it only says how copying works in the reader.
fn reader_keys(app: &mut App, keys: Vec<model::Key>) -> bool {
    let mut redraw = false;
    for key in keys {
        if matches!(key, model::Key::SeatNext | model::Key::SeatPrev) {
            let next = if key == model::Key::SeatNext {
                app.reader.index.checked_add(1)
            } else {
                app.reader.index.checked_sub(1)
            };
            if let Some(next) = next.filter(|next| *next < app.reader.files.len()) {
                app.read_file(app.reader.files.clone(), next, app.reader.base.clone());
                redraw = true;
            }
            continue;
        }
        if key == model::Key::Mode {
            if app.reader.document.is_some() {
                if let Some(why) = app.reader.rendered.as_ref().and_then(|(_, r)| r.refusal()) {
                    app.flash = Some((why.to_owned(), Instant::now()));
                } else {
                    app.reader.anchor =
                        Some(app.reader.source_line(app.model.reader_top().unwrap_or(0)));
                    app.reader.raw = !app.reader.raw;
                }
                redraw = true;
            }
            continue;
        }
        if key == model::Key::Copy {
            app.flash = Some(("drag to copy text".to_owned(), Instant::now()));
            redraw = true;
            continue;
        }
        redraw |= app.model.key(key, &app.fleet, false) == model::Act::Redraw;
    }
    if !app.model.reader_open() {
        app.close_reader();
        redraw = true;
    }
    redraw
}

/// One Text byte with the overlay open: `,` closes with its request, the
/// rest decodes modal. The overlay is open on entry; the caller resumes the
/// actual mode after a close.
fn settings_byte(app: &mut App, byte: u8) -> bool {
    if byte == b',' {
        app.close_settings();
        return true;
    }
    if byte == b'?' {
        app.model.show_settings_tab(model::SettingsTab::Keys);
        return true;
    }
    let mut redraw = false;
    for key in model::browse_keys(&Key::Text(vec![byte])) {
        redraw |= settings_act(app, key);
    }
    if !app.model.settings_open() {
        app.close_settings();
        redraw = true;
    }
    redraw
}

/// One decoded key with the overlay open: whether it changed the screen.
fn settings_act(app: &mut App, key: model::Key) -> bool {
    match app.model.key(key, &app.fleet, app.can_compose()) {
        model::Act::Redraw => true,
        model::Act::StatsScope => {
            app.flip_stats_scope();
            true
        }
        model::Act::StatsReset => {
            app.ask_reset();
            true
        }
        model::Act::StatsPage(next) => app.stats_page(next),
        model::Act::StatsFilter(next) => app.stats_filter(next),
        _ => false,
    }
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
