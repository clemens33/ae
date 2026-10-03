//! `ae app`: the Side Quiet layout — the fleet in a sidebar, the selected
//! session's Overview or Agents beside it, and the selected session's chat.
//!
//! The pure halves live in the submodules: [`fleet`] folds the sidebar rows,
//! [`overview`] the Overview tab, [`model`] the browse reducer and [`draw`] the
//! cells. Each reads only what the existing owners already computed; this
//! file is where the world is read, through those owners, and nothing here
//! writes into any session.

use std::collections::BTreeMap;
use std::io::{IsTerminal as _, Write};
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use ratatui_core::buffer::Buffer;
use ratatui_core::terminal::Terminal;

use crate::console::input::{Key, Keys};
use crate::console::lane::Lane;
use crate::console::needs::Section;
use crate::console::{self, Console};
use crate::digest::{SessionEntry, Status};
use crate::inventory::ServerId;
use crate::listing::World;
use crate::time::Timestamp;
use crate::{doors, store, theme, tmux, transport};

mod backend;
pub mod draw;
pub mod fleet;
mod lane;
pub mod model;
pub mod overview;
mod tty;

/// What `ae app` says, on stderr, when it has no terminal to draw on.
pub const NO_TERMINAL: &str = "ae app draws a terminal UI; use ae chat to print the lane\n";

/// The usage text.
pub const USAGE: &str = "Usage: ae app [session]\n\n  session   the home session, whose chat you write to (default: the session this pane belongs to)\n";

/// How often the fleet is read again.
const REFRESH: Duration = Duration::from_secs(crate::board::follow::POLL_SECS);
/// How long a wait for a key lasts before the size and the Esc bound are read.
const TICK: Duration = Duration::from_millis(100);
/// The most sessions whose lane is kept read.
const KEPT: usize = 3;

/// Everything the app has read, and the browse state.
struct App {
    root: PathBuf,
    home: Option<String>,
    server: Option<ServerId>,
    model: model::Model,
    fleet: fleet::Fleet,
    world: World,
    dirs: BTreeMap<String, PathBuf>,
    facts: BTreeMap<String, fleet::Facts>,
    needs: BTreeMap<String, Section>,
    /// The sessions whose lanes are followed, the most recently viewed first.
    consoles: Vec<Console>,
    lane: Lane,
    overview: overview::Overview,
    pair: Vec<String>,
    look: Option<theme::Look>,
    zone: Option<String>,
}

impl App {
    fn new(root: PathBuf, home: Option<String>) -> Self {
        let declared = doors::declared_server(crate::shape::current());
        let (look, zone) = home
            .as_deref()
            .map_or((None, None), |home| console::look_of(home, true));
        Self {
            root,
            home,
            server: doors::launch_target(declared.as_ref()),
            model: model::Model::default(),
            fleet: fleet::Fleet::default(),
            world: World::new(Timestamp::now(), Vec::new()),
            dirs: BTreeMap::new(),
            facts: BTreeMap::new(),
            needs: BTreeMap::new(),
            consoles: Vec::new(),
            lane: Lane::default(),
            overview: overview::Overview::default(),
            pair: Vec::new(),
            look,
            zone,
        }
    }

    /// Read the fleet again: `ae list`'s world, the picker's seat facts, the
    /// stopped sessions' last sign of life, the needs of every session that
    /// asks for attention, then the selected session.
    fn refresh(&mut self) {
        let now = Timestamp::now();
        let (snapshot, world) = crate::current_world(&self.root);
        self.dirs = snapshot
            .sessions
            .iter()
            .filter_map(|session| {
                let durable = session.candidate.durable.as_ref()?;
                Some((session.candidate.name.clone(), durable.path.clone()))
            })
            .collect();
        let picker = self
            .server
            .as_ref()
            .and_then(transport::observe_picker_sessions);
        self.facts = world
            .sessions
            .iter()
            .filter(|entry| entry.status != Status::Stopped)
            .map(|entry| (entry.name.clone(), facts_of(entry, picker.as_deref(), now)))
            .collect();
        let last_live: BTreeMap<String, i64> = world
            .sessions
            .iter()
            .filter(|entry| entry.status == Status::Stopped)
            .filter_map(|entry| {
                let dir = self.dirs.get(&entry.name)?;
                match crate::inventory::last_live(dir) {
                    tmux::Evidence::At(epoch) => Some((entry.name.clone(), epoch)),
                    _ => None,
                }
            })
            .collect();
        self.needs = world
            .sessions
            .iter()
            .filter(|entry| entry.attention.is_some() && entry.status != Status::Stopped)
            .filter_map(|entry| {
                let dir = self.dirs.get(&entry.name)?;
                let section = console::needs_of(&entry.name, dir).ok()?;
                Some((entry.name.clone(), section))
            })
            .collect();
        let order = crate::fleet_order();
        let home = self.home.as_deref();
        self.fleet = fleet::rows(
            &world,
            &self.facts,
            &last_live,
            &order,
            &self.needs,
            home,
            now,
        );
        self.world = world;
        if self.model.selected().is_none() {
            self.model = model::Model::new(&self.fleet);
        }
        self.model.reconcile(&self.fleet);
        self.view();
    }

    /// Read the selected session: its lane through its console, its needs,
    /// its memo for the Overview, and the home lead pair.
    fn view(&mut self) {
        let now = Timestamp::now();
        let Some(name) = self.model.selected().map(str::to_owned) else {
            return;
        };
        let Some(dir) = self.dirs.get(&name).cloned() else {
            return;
        };
        let at = self
            .consoles
            .iter()
            .position(|console| console.name() == name);
        let mut console = match at {
            Some(at) => self.consoles.remove(at),
            None => Console::open(name.clone(), dir.clone()),
        };
        match console.read() {
            Ok(read) => {
                self.lane = read.lane;
                if let Ok(section) = read.needs {
                    self.needs.insert(name.clone(), section);
                }
            }
            Err(why) => {
                self.lane = Lane {
                    items: Vec::new(),
                    coverage: vec![why],
                };
            }
        }
        self.consoles.insert(0, console);
        self.consoles.truncate(KEPT);
        if let Some(home) = &self.home {
            let home = self.consoles.iter().find(|console| console.name() == home);
            let seats = home.map(Console::seats).and_then(Result::ok);
            self.pair = lead_pair(seats.unwrap_or_default());
        }
        let memo = store::open(&dir)
            .memo_bytes()
            .map_err(|err| format!("memo unreadable ({err})"));
        let empty = SessionEntry::new(&name, Status::Unknown);
        let entry = self.entry(&name).unwrap_or(&empty);
        let memo = memo.as_deref().map_err(String::clone);
        self.overview = overview::of(entry, self.needs.get(&name), memo, now);
    }

    fn entry(&self, name: &str) -> Option<&SessionEntry> {
        self.world.sessions.iter().find(|entry| entry.name == name)
    }

    /// The composer line for the selection.
    fn composer(&self) -> draw::Composer<'_> {
        let speaker = self.pair.first().map_or("", String::as_str);
        match &self.home {
            None => draw::Composer::NoHome,
            Some(home) if self.model.selected() != Some(home.as_str()) => {
                draw::Composer::Foreign { home, speaker }
            }
            Some(_) => draw::Composer::ReadOnly {
                why: "this app reads only",
            },
        }
    }

    /// One frame into `buf`; the answer bounds the chat scroll.
    fn draw(&self, buf: &mut Buffer) -> usize {
        let selected = self.model.selected().and_then(|name| self.entry(name));
        let agents = self.model.selected().and_then(|name| self.facts.get(name));
        let screen = draw::Screen {
            fleet: &self.fleet,
            model: &self.model,
            overview: &self.overview,
            selected,
            pair: &self.pair,
            agents,
            lane: &self.lane,
            composer: self.composer(),
            look: self.look,
            zone: self.zone.as_deref(),
            now: Timestamp::now(),
        };
        draw::draw(&screen, buf)
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

/// The lead pair's names, main first.
fn lead_pair(mut seats: Vec<crate::console::lane::Seat>) -> Vec<String> {
    seats.sort_by_key(|seat| seat.slot != "main");
    seats.into_iter().map(|seat| seat.name).collect()
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
    let tty = match tty::Tty::start() {
        Ok(tty) => tty,
        Err(why) => {
            writeln!(err, "ae app: {why}")?;
            return Ok(crate::EXIT_UNAVAILABLE);
        }
    };
    let mut size = tty.size().unwrap_or((80, 24));
    let mut terminal = Terminal::new(backend::Ansi::new(out, size))?;
    let mut app = App::new(root, home);
    let mut keys = Keys::app();
    let mut due = Instant::now();
    let mut dirty = true;
    loop {
        if let Some(now) = tty.size().filter(|now| *now != size) {
            size = now;
            terminal.backend_mut().resize(size);
            dirty = true;
        }
        if Instant::now() >= due {
            app.refresh();
            due = Instant::now() + REFRESH;
            dirty = true;
        }
        if dirty {
            let mut pages = 0;
            terminal.draw(|frame| pages = app.draw(frame.buffer_mut()))?;
            app.model.clamp_pages(pages);
            dirty = false;
        }
        let wait = due.saturating_duration_since(Instant::now()).min(TICK);
        let keyed = match reads.recv_timeout(wait) {
            Ok((stamp, bytes)) => keys.feed(&bytes, stamp),
            Err(RecvTimeoutError::Timeout) => keys.idle(Instant::now()),
            Err(RecvTimeoutError::Disconnected) => break,
        };
        for (key, _) in keyed {
            match browse(&mut app, &key) {
                Some(true) => dirty = true,
                Some(false) => {}
                None => {
                    drop(tty);
                    return Ok(0);
                }
            }
        }
    }
    drop(tty);
    Ok(0)
}

/// Take one key: `Some(redraw)`, or `None` to quit.
fn browse(app: &mut App, key: &Key) -> Option<bool> {
    let mut redraw = false;
    for key in model::browse_keys(key) {
        match app.model.key(key, &app.fleet, false, true) {
            model::Act::Quit => return None,
            model::Act::Select(_) => {
                app.view();
                redraw = true;
            }
            model::Act::Redraw => redraw = true,
            model::Act::Compose | model::Act::None => {}
        }
    }
    Some(redraw)
}
