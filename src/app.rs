//! `ae app`: the Side Quiet layout — the fleet in a sidebar, the selected
//! session's Overview or Agents beside it, and the selected session's chat.
//!
//! The pure halves live in the submodules: [`fleet`] folds the sidebar rows,
//! [`overview`] the Overview tab, [`model`] the browse reducer and [`draw`] the
//! cells. Each reads only what the existing owners already computed; this
//! file is where the world is read, through those owners. The ONE write is
//! the home composer's ask or close, through the chat's own admission path
//! (`term::submit_ask`, `submit::close_owned`); nothing else here writes into
//! any session.

use std::collections::BTreeMap;
use std::io::{IsTerminal as _, Write};
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use ratatui_core::buffer::Buffer;
use ratatui_core::terminal::Terminal;

use crate::console::input::{Effect, Input, Key, Keys, Reading, Size, View};
use crate::console::lane::{Item, Kind, Lane, Seat};
use crate::console::needs::Section;
use crate::console::{self, Console, submit, term};
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
/// The most foreign sessions whose lane is kept read; home is always kept.
const KEPT: usize = 3;
/// The most of ae's own notices the home lane shows.
const NOTICES: usize = 5;

/// Everything the app has read, and the browse and compose state.
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
    /// The home session's console, never evicted: the pair, the ownership
    /// reading and every ask go through it.
    home_console: Option<Console>,
    /// Foreign sessions whose lanes are followed, the most recently viewed first.
    consoles: Vec<Console>,
    lane: Lane,
    overview: overview::Overview,
    /// The home lead pair as its meta names it now, main first.
    pair: Vec<String>,
    look: Option<theme::Look>,
    zone: Option<String>,
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
    draft_view: View,
    draft: String,
}

impl App {
    fn new(root: PathBuf, home: Option<String>) -> Self {
        let declared = doors::declared_server(crate::shape::current());
        let home_console = home.as_ref().and_then(|name| {
            let dir = console::locate(&root, name)?;
            Some(Console::open_standing(name.clone(), dir))
        });
        let pair = home_console
            .as_ref()
            .map(|console| console.seats().and_then(term::pair_of));
        let (seats, read_only) = match pair {
            Some(Ok(seats)) => (seats, "ownership not read yet".to_owned()),
            Some(Err(why)) => (Vec::new(), format!("input off: {why}")),
            None => (Vec::new(), String::new()),
        };
        let input = (!seats.is_empty())
            .then(|| Input::new(seats.iter().map(|seat| seat.name.clone()).collect()));
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
            home_console,
            consoles: Vec::new(),
            lane: Lane::default(),
            overview: overview::Overview::default(),
            pair: Vec::new(),
            look: None,
            zone: None,
            seats,
            input,
            me: doors::calling_pane_id(),
            read_only,
            composing: false,
            notices: Vec::new(),
            draft_view: View {
                rows: Vec::new(),
                cursor_row: 0,
                before: String::new(),
                anchor: String::new(),
            },
            draft: String::new(),
        }
    }

    /// Read the fleet again: `ae list`'s world, the picker's seat facts, the
    /// stopped sessions' last sign of life, the needs of every session that
    /// asks for attention, the look, who owns the input, then the selection.
    fn refresh(&mut self) {
        let now = Timestamp::now();
        let (snapshot, world) = crate::current_world(&self.root);
        let dirs = snapshot
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
        self.absorb(dirs, world, picker.as_deref(), &crate::fleet_order(), now);
        self.dress();
        self.own();
        self.view();
    }

    /// Fold one read of the fleet: the seat facts of every live session, the
    /// stopped ones' last sign of life, the needs of every live session that
    /// asks for attention, the sidebar rows, the selection and the lead pair.
    fn absorb(
        &mut self,
        dirs: BTreeMap<String, PathBuf>,
        world: World,
        picker: Option<&[tmux::PickerSession]>,
        order: &theme::FleetOrder,
        now: Timestamp,
    ) {
        self.dirs = dirs;
        self.facts = world
            .sessions
            .iter()
            .filter(|entry| entry.status != Status::Stopped)
            .map(|entry| (entry.name.clone(), facts_of(entry, picker, now)))
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
        let home = self.home.as_deref();
        self.fleet = fleet::rows(
            &world,
            &self.facts,
            &last_live,
            order,
            &self.needs,
            home,
            now,
        );
        self.world = world;
        if self.model.selected().is_none() {
            self.model = model::Model::new(&self.fleet);
        }
        self.model.reconcile(&self.fleet);
        if let Some(console) = &self.home_console {
            let seats = console.seats().and_then(term::pair_of);
            self.pair = seats.map_or_else(|_| self.pair.clone(), |seats| names(&seats));
        }
    }

    /// The look the chat reads (F3): the home session's drawn look, else the
    /// selected one's, else the default drawn look — re-read every refresh.
    fn dress(&mut self) {
        let selected = self.model.selected().map(str::to_owned);
        let mut first_zone = None;
        for name in self.home.iter().cloned().chain(selected) {
            let (look, zone) = console::look_of(&name, true);
            if look.is_some() {
                (self.look, self.zone) = (look, zone);
                return;
            }
            first_zone = first_zone.or(zone);
        }
        (self.look, self.zone) = (Some(theme::Look::DEFAULT), first_zone);
    }

    /// Read who owns the home session's input and hand it to the composer.
    fn own(&mut self) {
        let (Some(console), Some(_)) = (&self.home_console, &self.input) else {
            return;
        };
        let reading = match term::owns(console, self.server.as_ref(), self.me.as_deref()) {
            None => Reading::Unknown,
            Some(Ok(())) => Reading::Owner,
            Some(Err(why)) => Reading::NotOwner(why),
        };
        self.take(reading, Instant::now());
    }

    /// Hand one ownership reading to the composer at `now`; a promotion puts
    /// the kept draft back, as the chat does.
    fn take(&mut self, reading: Reading, now: Instant) {
        let (Some(console), Some(input)) = (&self.home_console, &mut self.input) else {
            return;
        };
        self.read_only = match &reading {
            Reading::NotOwner(why) => why.clone(),
            Reading::Unknown => UNREAD.to_owned(),
            Reading::Owner => String::new(),
        };
        let was = input.taking();
        let _ = input.tick(reading, now);
        if !was && input.taking() {
            let restored = input.restore(submit::restore(console.dir()));
            self.effects(restored);
        }
        if !self.input.as_ref().is_some_and(Input::taking) {
            self.composing = false;
        }
    }

    /// Read the selected session: its lane through its console, its needs,
    /// its memo for the Overview. Nothing of an earlier selection survives a
    /// read that fails.
    fn view(&mut self) {
        let now = Timestamp::now();
        let Some(name) = self.model.selected().map(str::to_owned) else {
            (self.lane, self.overview) = (Lane::default(), overview::Overview::default());
            return;
        };
        let empty = SessionEntry::new(&name, Status::Unknown);
        let Some(dir) = self.dirs.get(&name).cloned() else {
            let gap = format!("{name} has no record directory ae can read");
            self.lane = Lane {
                items: Vec::new(),
                coverage: vec![gap.clone()],
            };
            let entry = self.entry(&name).unwrap_or(&empty).clone();
            self.overview = overview::of(&entry, None, Err(gap), now);
            return;
        };
        let home = self.home.as_deref() == Some(name.as_str());
        let console = if home {
            self.home_console.as_mut()
        } else {
            let at = self
                .consoles
                .iter()
                .position(|console| console.name() == name);
            let console = match at {
                Some(at) => self.consoles.remove(at),
                None => Console::open_standing(name.clone(), dir.clone()),
            };
            self.consoles.insert(0, console);
            self.consoles.truncate(KEPT);
            self.consoles.first_mut()
        };
        let read = console.map(Console::read);
        self.lane = match read {
            Some(Ok(read)) => {
                let mut lane = read.lane;
                match read.needs {
                    Ok(section) => drop(self.needs.insert(name.clone(), section)),
                    Err(why) => lane.coverage.push(format!("needs you unread: {why}")),
                }
                lane
            }
            Some(Err(why)) => Lane {
                items: Vec::new(),
                coverage: vec![why],
            },
            None => Lane::default(),
        };
        if home {
            // The seats `/open` names, as the chat's own tick hands them over.
            let roster = self.home_console.as_ref().and_then(Console::roster);
            if let (Some(seats), Some(input)) = (roster, self.input.as_mut()) {
                input.set_seats(seats.to_vec());
            }
            self.lane.items.extend(self.notices.iter().cloned());
            self.lane.items.sort_by_key(|item| item.micros);
        }
        let memo = store::open(&dir)
            .memo_bytes()
            .map_err(|err| format!("memo unreadable ({err})"));
        let entry = self.entry(&name).unwrap_or(&empty);
        let memo = memo.as_deref().map_err(String::clone);
        self.overview = overview::of(entry, self.needs.get(&name), memo, now);
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
                    self.view();
                }
            }
        }
        Some(())
    }

    /// Carry out what the input asked for; each outcome becomes a notice.
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
        }
        let over = self.notices.len().saturating_sub(NOTICES);
        self.notices.drain(..over);
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
    fn frame(&mut self, buf: &mut Buffer) {
        if let (Some(input), Some(home)) = (&self.input, &self.home) {
            let area = buf.area;
            let width = draw::draft_width(area.width, area.height, home, input.speaker());
            let size = Size {
                width,
                height: usize::from(area.height),
            };
            (self.draft_view, self.draft) = (input.bare_view(size), input.draft());
        }
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
        let pages = draw::draw(&screen, buf);
        self.model.clamp_pages(pages);
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
            terminal.draw(|frame| app.frame(frame.buffer_mut()))?;
            dirty = false;
        }
        let wait = due.saturating_duration_since(Instant::now()).min(TICK);
        let keyed = match reads.recv_timeout(wait) {
            Ok((stamp, bytes)) => keys.feed(&bytes, stamp),
            Err(RecvTimeoutError::Timeout) => keys.idle(Instant::now()),
            Err(RecvTimeoutError::Disconnected) => break,
        };
        for (key, origin) in keyed {
            let next = if app.composing {
                app.compose(key, origin).map(|()| true)
            } else {
                browse(&mut app, &key)
            };
            let Some(redraw) = next else {
                drop(tty);
                return Ok(0);
            };
            dirty |= redraw;
        }
    }
    drop(tty);
    Ok(0)
}

/// Take one key: `Some(redraw)`, or `None` to quit.
fn browse(app: &mut App, key: &Key) -> Option<bool> {
    let mut redraw = false;
    for key in model::browse_keys(key) {
        match app.model.key(key, &app.fleet, app.can_compose(), true) {
            model::Act::Quit => return None,
            model::Act::Select(_) => {
                app.dress();
                app.view();
                redraw = true;
            }
            model::Act::Redraw => redraw = true,
            model::Act::Compose => {
                app.composing = true;
                redraw = true;
            }
            model::Act::None => {}
        }
    }
    Some(redraw)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::Rect;

    use std::collections::BTreeMap;

    use super::{App, UNREAD, USAGE, browse, facts_of, fleet, run};
    use crate::app::fleet::{Counts, Fleet, Line2, Row};
    use crate::app::model::{Key as Browse, Model};
    use crate::attention::Reason;
    use crate::console::input::{Input, Key, Reading};
    use crate::console::lane::{Item, Kind, Lane};
    use crate::digest::{SessionEntry, Status};
    use crate::listing::World;
    use crate::theme::{FleetOrder, Mark};
    use crate::time::Timestamp;
    use crate::tmux;

    const ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";

    /// A state root holding one lead-pair session, `api`, removed on drop.
    struct Root(PathBuf);
    impl Root {
        fn new(tag: &str) -> Self {
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
            rows: vec![Row {
                name: "api".to_owned(),
                index: 1,
                mark: Mark::Working,
                needy: false,
                counts: Counts::Unknown,
                line2: Line2::NoGoal,
                home: home.is_some(),
            }],
            home: home.map(str::to_owned),
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

    /// B2-F4: a frame bounds the scroll to the pages the lane holds — the
    /// bound is the FIRST page count that shows the oldest turn, so one page
    /// fewer hides it — and a lane that fits scrolls nowhere.
    #[test]
    fn a_frame_bounds_the_scroll_to_the_oldest_turn() {
        let root = Root::new("scroll");
        let mut app = App::new(root.0.clone(), None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        let mut items = vec![said("oldest turn", 0)];
        items.extend((1..60).map(|at| said(&format!("turn {at}"), at)));
        app.lane = Lane {
            items,
            coverage: Vec::new(),
        };
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
        app.lane.items.truncate(2);
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
        let mut app = App::new(root.0.clone(), Some("api".to_owned()));
        app.server = None;
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

    /// A redrawn frame keeps the coverage that still stands: the board's
    /// follow reports a gap once (the chat prints it once), and every later
    /// read of the same session, home or foreign, still shows it.
    #[test]
    fn standing_coverage_survives_every_later_read() {
        let root = Root::new("coverage");
        let mut app = App::new(root.0.clone(), Some("api".to_owned()));
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.dirs
            .insert("api".to_owned(), root.0.join("sessions").join("api"));
        app.view();
        let first = app.lane.coverage.clone();
        assert!(
            first.iter().any(|row| row.contains("api:lead")),
            "a seat with no conversation is a gap: {first:?}"
        );
        app.view();
        assert_eq!(app.lane.coverage, first, "the home gap still stands");
        app.home = None;
        app.home_console = None;
        app.view();
        app.view();
        assert_eq!(app.lane.coverage, first, "a foreign view keeps it too");
    }

    /// `/open` in the app names why it opens nothing, as the docs say, once
    /// the home roster is read: the app selects no pane, `ae chat` does.
    #[test]
    fn open_is_refused_with_the_apps_own_reason() {
        let root = Root::new("open");
        let mut app = App::new(root.0.clone(), Some("api".to_owned()));
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.dirs
            .insert("api".to_owned(), root.0.join("sessions").join("api"));
        app.view();
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
    fn session(root: &Root, name: &str, meta_tail: &str) -> PathBuf {
        let dir = root.0.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("session dir");
        let meta = format!(
            "schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n{meta_tail}"
        );
        std::fs::write(dir.join("meta"), meta).expect("meta");
        dir
    }

    fn entry(name: &str, status: Status, attention: Option<Reason>) -> SessionEntry {
        let mut entry = SessionEntry::new(name, status);
        entry.attention = attention;
        entry
    }

    /// #9-13, #37-39: the fold over one read. Seat facts for every session
    /// still running, a last sign of life for every stopped one (its own
    /// launch row), needs only for a running session that asks for
    /// attention, and the home pair as the header names it.
    #[test]
    fn one_read_folds_facts_life_and_needs_by_status() {
        const LAUNCHED: i64 = 1_759_000_000;
        let root = Root::new("absorb");
        let dirs: BTreeMap<String, PathBuf> = [
            ("run", ""),
            ("stop", &*format!("launch_time.main={LAUNCHED}\n")),
            ("unk", ""),
        ]
        .into_iter()
        .map(|(name, tail)| (name.to_owned(), session(&root, name, tail)))
        .collect();
        let world = World::new(
            Timestamp::now(),
            vec![
                entry("run", Status::Running, Some(Reason::Blocked)),
                entry("stop", Status::Stopped, Some(Reason::Blocked)),
                entry("unk", Status::Unknown, None),
            ],
        );
        let mut app = App::new(root.0.clone(), Some("api".to_owned()));
        app.absorb(dirs, world, None, &FleetOrder::EMPTY, Timestamp::now());
        let keys = |map: Vec<&String>| map.into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(keys(app.facts.keys().collect()), ["run", "unk"]);
        assert_eq!(keys(app.needs.keys().collect()), ["run"]);
        let stopped = app.fleet.rows.iter().find(|row| row.name == "stop");
        assert!(
            matches!(
                stopped.map(|row| &row.line2),
                Some(Line2::NotRunning {
                    stopped_secs: Some(_),
                    ..
                })
            ),
            "the stopped row ages from its own launch row"
        );
        let rows = &app.needs["run"].rows;
        let seats: Vec<&str> = rows.iter().map(|row| row.seat.name.as_str()).collect();
        assert_eq!(
            seats,
            ["lead", "colead"],
            "every roster seat of the session"
        );
        let pair: Vec<bool> = rows.iter().map(|row| row.lead_pair).collect();
        assert_eq!(
            pair,
            [true, true],
            "a lead-pair session's worker.0 is of the pair"
        );
        assert_eq!(app.pair, ["lead", "colead"]);
    }

    /// #15: no server to ask means tmux did not answer who owns the input.
    #[test]
    fn an_unanswered_ownership_says_tmux_did_not_answer() {
        let root = Root::new("own");
        let mut app = App::new(root.0.clone(), Some("api".to_owned()));
        app.server = None;
        app.own();
        assert_eq!(app.read_only, UNREAD);
    }

    /// #16-18: only the promotion to owner brings the kept draft back, once;
    /// losing the input ends composing.
    #[test]
    fn only_a_promotion_brings_the_kept_draft_back_once() {
        let root = Root::new("take");
        let dir = root.0.join("sessions").join("api");
        crate::store::open(&dir)
            .publish_console_draft(b"kept line")
            .expect("a kept draft");
        let mut app = App::new(root.0.clone(), Some("api".to_owned()));
        let at = Instant::now();
        let draft = |app: &App| app.input.as_ref().map(Input::draft).unwrap_or_default();
        app.take(Reading::Unknown, at);
        assert_eq!(draft(&app), "");
        app.take(Reading::Owner, at);
        assert_eq!(draft(&app), "kept line", "the promotion restores");
        app.composing = true;
        app.take(Reading::Owner, at);
        assert_eq!(draft(&app), "kept line", "no second restore");
        assert!(app.composing, "the owner keeps composing");
        app.take(Reading::NotOwner("owned by window @2".to_owned()), at);
        assert!(!app.composing, "losing the input ends composing");
    }

    /// #19: each viewed foreign session reads through its own console.
    #[test]
    fn each_viewed_session_reads_through_its_own_console() {
        let root = Root::new("cache");
        let mut app = App::new(root.0.clone(), None);
        app.fleet = Fleet {
            rows: ["web", "ops"]
                .into_iter()
                .enumerate()
                .map(|(at, name)| Row {
                    name: name.to_owned(),
                    index: at + 1,
                    ..one_row(None).rows[0].clone()
                })
                .collect(),
            home: None,
        };
        for name in ["web", "ops"] {
            app.dirs.insert(name.to_owned(), session(&root, name, ""));
        }
        app.model = Model::new(&app.fleet);
        for (digit, viewed, other) in [(1, "web", "ops"), (2, "ops", "web"), (1, "web", "ops")] {
            let _ = app.model.key(Browse::Digit(digit), &app.fleet, false, true);
            app.view();
            let coverage = app.lane.coverage.join("\n");
            assert!(
                coverage.contains(&format!("{viewed}:lead")),
                "{viewed}: {coverage}"
            );
            assert!(
                !coverage.contains(&format!("{other}:")),
                "{viewed}: {coverage}"
            );
        }
    }

    /// #23/#25: an entry is found by its own name, and a selected running
    /// session draws its tabs (frame calm r18).
    #[test]
    fn an_entry_is_found_by_its_own_name() {
        let root = Root::new("entry");
        let mut app = App::new(root.0.clone(), None);
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
        let mut app = App::new(root.0.clone(), Some("api".to_owned()));
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
        let mut app = App::new(root.0.clone(), Some("api".to_owned()));
        let mut fleet = one_row(Some("api"));
        fleet.rows.push(Row {
            name: "web".to_owned(),
            index: 2,
            home: false,
            ..fleet.rows[0].clone()
        });
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
        let mut app = App::new(root.0.clone(), Some("api".to_owned()));
        app.server = None;
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
        let root = Root::new("browse");
        let mut app = App::new(root.0.clone(), None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        assert!(browse(&mut app, &Key::Text(b"x".to_vec())).is_some());
        assert!(browse(&mut app, &Key::Text(b"q".to_vec())).is_none());
    }
}
