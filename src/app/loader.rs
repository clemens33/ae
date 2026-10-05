//! The one background reader of `ae app`: every read of the world the app
//! draws — the fleet, the looks, who owns the home input, each session's lane
//! — happens on this thread, so a key never waits behind one. The UI thread
//! owns the model and the frame and folds what arrives here.
//!
//! Each answer names what it describes: a look or a lane is tagged with its
//! session (and a lane with the identity its console is bound to and a read
//! sequence), so nothing read for one session can be drawn under another.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Instant;

use super::{REFRESH, facts_of, fleet, names};
use crate::console::input::Reading;
use crate::console::lane::{Lane, Seat};
use crate::console::needs::{SeatRef, Section};
use crate::console::{self, Console, submit, term};
use crate::digest::Status;
use crate::inventory::ServerId;
use crate::listing::World;
use crate::time::Timestamp;
use crate::{store, theme, tmux, transport};

/// The most sessions whose consoles are kept read, home and the selection
/// among them.
pub(super) const KEEP: usize = 16;

/// What the UI asks of the reader.
pub(super) enum Request {
    /// The session the UI shows now.
    Focus(Option<String>),
    /// Read this session again: the UI wrote into it.
    Reread(String),
}

/// What wakes the UI: keys, the end of its input, or an answer.
pub(super) enum Wake {
    Keys(Instant, Vec<u8>),
    Closed,
    Answer(Box<Answer>),
}

/// One answer from the reader.
#[allow(
    clippy::large_enum_variant,
    reason = "every answer travels boxed in `Wake::Answer`; boxing a variant too buys nothing"
)]
pub(super) enum Answer {
    /// The home session, opened: the console its writes go through and its
    /// lead pair, or `None` when there is no home to open.
    Home(Option<(Console, Result<Vec<Seat>, String>)>),
    Fleet(Box<FleetRead>),
    /// Session `name`'s drawn look and the viewer's zone.
    Look {
        name: String,
        look: Option<theme::Look>,
        zone: Option<String>,
    },
    /// Who owns the home input, read at `at`; `draft` is the kept draft read
    /// with an owner reading that followed a reading that was not one.
    Owned {
        reading: Reading,
        at: Instant,
        draft: Option<submit::Draft>,
    },
    View(Box<ViewRead>),
}

/// One read of the fleet.
pub(super) struct FleetRead {
    pub(super) dirs: BTreeMap<String, PathBuf>,
    /// Each session's recorded `session_id`, canonical or empty.
    pub(super) ids: BTreeMap<String, String>,
    pub(super) world: World,
    pub(super) facts: BTreeMap<String, fleet::Facts>,
    pub(super) needs: BTreeMap<String, Section>,
    pub(super) fleet: fleet::Fleet,
    /// The home lead pair as its meta names it now; `None` keeps the last.
    pub(super) pair: Option<Vec<String>>,
    pub(super) memos: BTreeMap<String, Result<Vec<u8>, String>>,
}

/// One read of session `name` through its console.
pub(super) struct ViewRead {
    pub(super) name: String,
    /// The `session_id` the console is bound to.
    pub(super) id: String,
    pub(super) seq: u64,
    pub(super) lane: Lane,
    pub(super) needs: Option<Section>,
    /// The home roster `/open` names, after a settled home read.
    pub(super) roster: Option<Vec<SeatRef>>,
}

/// How eagerly a session is kept read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tier {
    /// The selection and home: read again every refresh.
    Hot,
    /// The sidebar neighbours, two each side: read again only once stale
    /// AND older than the last change of selection.
    Warm,
    /// The rest: read once.
    Cold,
}

/// The sessions to keep read, most wanted first: the selection, home, the
/// neighbours one then two rows away (below before above), then the rest in
/// sidebar order — at most [`KEEP`].
pub(super) fn order(
    rows: &[String],
    selected: Option<&str>,
    home: Option<&str>,
) -> Vec<(String, Tier)> {
    let mut out: Vec<(String, Tier)> = Vec::new();
    let mut add = |name: &str, tier| {
        if out.len() < KEEP && !out.iter().any(|(kept, _)| kept == name) {
            out.push((name.to_owned(), tier));
        }
    };
    selected
        .into_iter()
        .chain(home)
        .for_each(|name| add(name, Tier::Hot));
    if let Some(at) = selected.and_then(|name| rows.iter().position(|row| row == name)) {
        for step in [1, 2] {
            let below = rows.get(at + step);
            let above = at.checked_sub(step).and_then(|at| rows.get(at));
            below
                .into_iter()
                .chain(above)
                .for_each(|name| add(name, Tier::Warm));
        }
    }
    for name in rows {
        add(name, Tier::Cold);
    }
    out
}

/// What the reader does next.
enum Job {
    Fleet,
    Look(String),
    View(String),
    Wait(Instant),
}

/// Everything the reader holds between reads.
pub(super) struct Reader {
    root: PathBuf,
    home: Option<String>,
    server: Option<ServerId>,
    me: Option<String>,
    home_console: Option<Console>,
    consoles: BTreeMap<String, Console>,
    /// When each session's last read COMPLETED.
    read_at: BTreeMap<String, Instant>,
    dirs: BTreeMap<String, PathBuf>,
    ids: BTreeMap<String, String>,
    rows: Vec<String>,
    selected: Option<String>,
    focus_at: Instant,
    fleet_at: Option<Instant>,
    rereads: BTreeSet<String>,
    looks: BTreeSet<String>,
    /// Home's look is drawn, so no other session's look dresses the app.
    home_drawn: bool,
    /// The last ownership reading sent was an owner one.
    owner: bool,
    seq: u64,
}

/// Start the reader on its own thread; answers arrive on `wake`.
///
/// # Errors
///
/// Why the thread could not be started.
pub(super) fn spawn(
    reader: Reader,
    wake: Sender<Wake>,
) -> std::io::Result<(Sender<Request>, std::thread::JoinHandle<()>)> {
    let (ask, asks) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("ae-app-reader".to_owned())
        .spawn(move || reader.run(&asks, &wake))?;
    Ok((ask, handle))
}

impl Reader {
    pub(super) fn new(
        root: PathBuf,
        home: Option<String>,
        server: Option<ServerId>,
        me: Option<String>,
    ) -> Self {
        Self {
            root,
            home,
            server,
            me,
            home_console: None,
            consoles: BTreeMap::new(),
            read_at: BTreeMap::new(),
            dirs: BTreeMap::new(),
            ids: BTreeMap::new(),
            rows: Vec::new(),
            selected: None,
            focus_at: Instant::now(),
            fleet_at: None,
            rereads: BTreeSet::new(),
            looks: BTreeSet::new(),
            home_drawn: false,
            owner: false,
            seq: 0,
        }
    }

    /// Open home, then read until the UI is gone.
    fn run(mut self, asks: &Receiver<Request>, wake: &Sender<Wake>) {
        let send = |answer| wake.send(Wake::Answer(Box::new(answer))).is_ok();
        if !send(self.open_home()) {
            return;
        }
        loop {
            loop {
                match asks.try_recv() {
                    Ok(request) => self.take(request),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            let answers = match self.next(Instant::now()) {
                Job::Fleet => self.fleet(),
                Job::Look(name) => vec![self.look(&name)],
                Job::View(name) => self.view(&name).into_iter().collect(),
                Job::Wait(until) => {
                    match asks.recv_timeout(until.saturating_duration_since(Instant::now())) {
                        Ok(request) => self.take(request),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    Vec::new()
                }
            };
            if !answers.into_iter().all(send) {
                return;
            }
        }
    }

    /// Home's console and its lead pair, read once; a second console on the
    /// same session goes to the UI for its writes.
    pub(super) fn open_home(&mut self) -> Answer {
        let opened = self.home.clone().and_then(|name| {
            let dir = console::locate(&self.root, &name)?;
            Some((name, dir))
        });
        let Some((name, dir)) = opened else {
            return Answer::Home(None);
        };
        let writes = Console::open_standing(name.clone(), dir.clone());
        let pair = writes.seats().and_then(term::pair_of);
        self.home_console = Some(Console::open_standing(name, dir));
        Answer::Home(Some((writes, pair)))
    }

    pub(super) fn take(&mut self, request: Request) {
        match request {
            Request::Focus(selected) => {
                if let Some(name) = selected.as_ref().filter(|_| !self.home_drawn) {
                    self.looks.insert(name.clone());
                }
                self.selected = selected;
                self.focus_at = Instant::now();
                self.evict();
            }
            Request::Reread(name) => drop(self.rereads.insert(name)),
        }
    }

    /// Whether session `name` has a console to read through.
    fn readable(&self, name: &str) -> bool {
        if self.home.as_deref() == Some(name) {
            self.home_console.is_some()
        } else {
            self.dirs.contains_key(name)
        }
    }

    /// When session `name` of `tier` is next due, `None` for never.
    fn due(&self, name: &str, tier: Tier, now: Instant) -> Option<Instant> {
        let Some(at) = self.read_at.get(name) else {
            return Some(now);
        };
        match tier {
            Tier::Hot => Some(*at + REFRESH),
            Tier::Warm => (*at < self.focus_at).then_some(*at + REFRESH),
            Tier::Cold => None,
        }
    }

    /// The first job due at `now` by priority — the fleet until it is first
    /// read, a reread, a look, the selection, the fleet, home, the
    /// neighbours, the rest — else a wait until the earliest falls due.
    fn next(&self, now: Instant) -> Job {
        let Some(fleet_at) = self.fleet_at else {
            return Job::Fleet;
        };
        if let Some(name) = self.rereads.iter().find(|name| self.readable(name)) {
            return Job::View(name.clone());
        }
        if let Some(name) = self.looks.first() {
            return Job::Look(name.clone());
        }
        let mut order = order(&self.rows, self.selected.as_deref(), self.home.as_deref())
            .into_iter()
            .filter(|(name, _)| self.readable(name))
            .map(|(name, tier)| (Some(name.clone()), self.due(&name, tier, now)))
            .peekable();
        let mut candidates = Vec::new();
        if let Some(first) = order.next_if(|(name, _)| *name == self.selected) {
            candidates.push(first);
        }
        candidates.push((None, Some(fleet_at + REFRESH)));
        candidates.extend(order);
        if let Some((name, _)) = candidates
            .iter()
            .find(|(_, due)| due.is_some_and(|due| due <= now))
        {
            return name.clone().map_or(Job::Fleet, Job::View);
        }
        let until = candidates.iter().filter_map(|(_, due)| *due).min();
        Job::Wait(until.unwrap_or(now + REFRESH).min(now + REFRESH))
    }

    /// Read the fleet again: home's look (and the selection's when home's is
    /// not drawn), who owns the home input, then `ae list`'s world folded into
    /// the sidebar.
    pub(super) fn fleet(&mut self) -> Vec<Answer> {
        let mut answers: Vec<Answer> = self
            .home
            .clone()
            .map(|home| self.look(&home))
            .into_iter()
            .collect();
        if let Some(name) = self.selected.clone().filter(|_| !self.home_drawn) {
            answers.push(self.look(&name));
        }
        answers.extend(self.owned());
        let now = Timestamp::now();
        let (snapshot, world) = crate::current_world(&self.root);
        let dirs: BTreeMap<String, PathBuf> = snapshot
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
        let read = self.fold(dirs, world, picker.as_deref(), &crate::fleet_order(), now);
        self.fleet_at = Some(Instant::now());
        self.dirs.clone_from(&read.dirs);
        self.ids.clone_from(&read.ids);
        self.rows = read.fleet.rows.iter().map(|row| row.name.clone()).collect();
        self.evict();
        answers.push(Answer::Fleet(Box::new(read)));
        answers
    }

    /// Fold one read of the fleet: the seat facts of every live session, the
    /// stopped ones' last sign of life, the needs of every live session that
    /// asks for attention, each session's identity and memo, the sidebar rows
    /// and the home pair.
    pub(super) fn fold(
        &self,
        dirs: BTreeMap<String, PathBuf>,
        world: World,
        picker: Option<&[tmux::PickerSession]>,
        order: &theme::FleetOrder,
        now: Timestamp,
    ) -> FleetRead {
        let facts = world
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
                let dir = dirs.get(&entry.name)?;
                match crate::inventory::last_live(dir) {
                    tmux::Evidence::At(epoch) => Some((entry.name.clone(), epoch)),
                    _ => None,
                }
            })
            .collect();
        let needs = world
            .sessions
            .iter()
            .filter(|entry| entry.attention.is_some() && entry.status != Status::Stopped)
            .filter_map(|entry| {
                let dir = dirs.get(&entry.name)?;
                let section = console::needs_of(&entry.name, dir).ok()?;
                Some((entry.name.clone(), section))
            })
            .collect();
        let fleet = fleet::rows(
            &world,
            &facts,
            &last_live,
            order,
            &needs,
            self.home.as_deref(),
            now,
        );
        let ids = dirs
            .iter()
            .map(|(name, dir)| (name.clone(), console::recorded_uuid(dir)))
            .collect();
        let memos = dirs
            .iter()
            .map(|(name, dir)| {
                let memo = store::open(dir)
                    .memo_bytes()
                    .map_err(|err| format!("memo unreadable ({err})"));
                (name.clone(), memo)
            })
            .collect();
        let pair = self
            .home_console
            .as_ref()
            .and_then(|console| console.seats().and_then(term::pair_of).ok())
            .map(|seats| names(&seats));
        FleetRead {
            dirs,
            ids,
            world,
            facts,
            needs,
            fleet,
            pair,
            memos,
        }
    }

    /// Drop every foreign console no longer kept: out of the [`order`], gone
    /// from the fleet, moved, or bound to an identity its session no longer
    /// records. Its next read opens it afresh.
    fn evict(&mut self) {
        let keep: Vec<String> = order(&self.rows, self.selected.as_deref(), self.home.as_deref())
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let (dirs, ids) = (&self.dirs, &self.ids);
        self.consoles.retain(|name, console| {
            keep.contains(name)
                && dirs.get(name).is_some_and(|dir| dir == console.dir())
                && ids.get(name).is_some_and(|id| id == console.uuid())
        });
        let (home, consoles) = (self.home.as_deref(), &self.consoles);
        self.read_at
            .retain(|name, _| home == Some(name.as_str()) || consoles.contains_key(name));
    }

    /// Session `name`'s drawn look, and whether home's is drawn.
    pub(super) fn look(&mut self, name: &str) -> Answer {
        self.looks.remove(name);
        let (look, zone) = console::look_of(name, true);
        if self.home.as_deref() == Some(name) {
            self.home_drawn = look.is_some();
        }
        Answer::Look {
            name: name.to_owned(),
            look,
            zone,
        }
    }

    /// Who owns the home input, read now.
    pub(super) fn owned(&mut self) -> Option<Answer> {
        let console = self.home_console.as_ref()?;
        let reading = reading(console, self.server.as_ref(), self.me.as_deref());
        Some(self.carry(reading, Instant::now()))
    }

    /// `reading`, completed at `at`, as it goes to the UI: the kept draft
    /// rides along exactly when the reading can promote — an owner one after
    /// a not-owner one, as the composer counts them (unknown changes nothing).
    fn carry(&mut self, reading: Reading, at: Instant) -> Answer {
        let promotes = match &reading {
            Reading::Owner => !std::mem::replace(&mut self.owner, true),
            Reading::NotOwner(_) => {
                self.owner = false;
                false
            }
            Reading::Unknown => false,
        };
        let draft = self
            .home_console
            .as_ref()
            .filter(|_| promotes)
            .map(|console| submit::restore(console.dir()));
        Answer::Owned { reading, at, draft }
    }

    /// Read session `name` through its console: its lane, with any needs gap
    /// named in its coverage, its needs, and — for home — its roster.
    pub(super) fn view(&mut self, name: &str) -> Option<Answer> {
        self.rereads.remove(name);
        let home = self.home.as_deref() == Some(name);
        let console = if home {
            self.home_console.as_mut()?
        } else {
            let dir = self.dirs.get(name)?.clone();
            self.consoles
                .entry(name.to_owned())
                .or_insert_with(|| Console::open_standing(name.to_owned(), dir))
        };
        let (lane, needs) = match console.read() {
            Ok(read) => {
                let mut lane = read.lane;
                let needs = match read.needs {
                    Ok(section) => Some(section),
                    Err(why) => {
                        lane.coverage.push(format!("needs you unread: {why}"));
                        None
                    }
                };
                (lane, needs)
            }
            Err(why) => (
                Lane {
                    items: Vec::new(),
                    coverage: vec![why],
                },
                None,
            ),
        };
        let roster = home
            .then(|| console.roster().map(<[SeatRef]>::to_vec))
            .flatten();
        let id = console.uuid().to_owned();
        self.read_at.insert(name.to_owned(), Instant::now());
        self.seq += 1;
        Some(Answer::View(Box::new(ViewRead {
            name: name.to_owned(),
            id,
            seq: self.seq,
            lane,
            needs,
            roster,
        })))
    }
}

/// Who owns `console`'s session input, as the composer takes it.
pub(super) fn reading(console: &Console, server: Option<&ServerId>, me: Option<&str>) -> Reading {
    match term::owns(console, server, me) {
        None => Reading::Unknown,
        Some(Ok(())) => Reading::Owner,
        Some(Err(why)) => Reading::NotOwner(why),
    }
}

#[cfg(test)]
mod tests {
    //! Oracles: the brief's rulings 2, 3 and 7 (priority, cadence, the draft
    //! a promotion restores), the fixture root's own records and the existing
    //! owners' coverage words.

    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::{Answer, Job, KEEP, Reader, Request, Tier, order};
    use crate::app::REFRESH;
    use crate::app::fleet::Line2;
    use crate::app::tests::{ID, Root, entry, session};
    use crate::attention::Reason;
    use crate::console::input::Reading;
    use crate::console::submit::Draft;
    use crate::digest::Status;
    use crate::listing::World;
    use crate::theme::FleetOrder;
    use crate::time::Timestamp;

    fn rows(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn coverage(answer: Option<Answer>) -> Vec<String> {
        match answer {
            Some(Answer::View(view)) => view.lane.coverage,
            _ => panic!("a lane read"),
        }
    }

    fn viewing(job: &Job) -> Option<&str> {
        match job {
            Job::View(name) => Some(name),
            _ => None,
        }
    }

    /// Ruling 3: the selection, home, then the neighbours one then two rows
    /// away, then the rest in sidebar order, at most KEEP, each once.
    #[test]
    fn the_order_is_selection_home_neighbours_then_the_rest() {
        let fleet: Vec<String> = (0..20).map(|at| format!("s{at}")).collect();
        let kept = order(&fleet, Some("s5"), Some("s0"));
        let names: Vec<&str> = kept.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names.len(), KEEP);
        assert_eq!(
            names,
            [
                "s5", "s0", "s6", "s4", "s7", "s3", "s1", "s2", "s8", "s9", "s10", "s11", "s12",
                "s13", "s14", "s15"
            ]
        );
        let tiers: Vec<Tier> = kept.iter().map(|(_, tier)| *tier).collect();
        assert_eq!(tiers[..2], [Tier::Hot, Tier::Hot]);
        assert!(tiers[2..6].iter().all(|tier| *tier == Tier::Warm));
        assert!(tiers[6..].iter().all(|tier| *tier == Tier::Cold));
        let edge = order(&rows(&["a", "b", "c"]), Some("a"), Some("b"));
        assert_eq!(
            edge,
            [
                ("a".to_owned(), Tier::Hot),
                ("b".to_owned(), Tier::Hot),
                ("c".to_owned(), Tier::Warm)
            ],
            "home stays hot as a neighbour; nothing above the top row"
        );
        let unselected = order(&rows(&["a", "b"]), None, Some("b"));
        assert_eq!(
            unselected,
            [("b".to_owned(), Tier::Hot), ("a".to_owned(), Tier::Cold)]
        );
    }

    /// Ruling 3 and I4: the fleet before anything is read; then a reread, a
    /// look, the selection, the fleet, home, a never-read neighbour; each due
    /// a REFRESH after its read COMPLETED; a neighbour again only once stale
    /// AND older than the last change of selection; the rest never twice.
    #[test]
    fn each_read_falls_due_by_its_priority_and_completion() {
        let root = Root::new("next");
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None, None);
        let _ = reader.open_home();
        let now = Instant::now();
        assert!(matches!(reader.next(now), Job::Fleet), "nothing read yet");
        reader.fleet_at = Some(now);
        reader.rows = rows(&["api", "web", "ops", "far", "end", "cold"]);
        for name in ["web", "ops", "far", "end", "cold"] {
            reader.dirs.insert(name.to_owned(), PathBuf::from(name));
        }
        reader.take(Request::Focus(Some("ops".to_owned())));
        reader.focus_at = now;
        assert!(matches!(reader.next(now), Job::Look(ref name) if name == "ops"));
        reader.looks.clear();
        reader.rereads.insert("api".to_owned());
        assert_eq!(viewing(&reader.next(now)), Some("api"), "a reread first");
        reader.rereads.clear();
        for expected in ["ops", "api", "far", "web", "end", "cold"] {
            assert_eq!(viewing(&reader.next(now)), Some(expected));
            reader.read_at.insert(expected.to_owned(), now);
        }
        let later = now + REFRESH;
        assert!(matches!(reader.next(now), Job::Wait(until) if until == later));
        assert_eq!(viewing(&reader.next(later)), Some("ops"), "selection first");
        reader
            .read_at
            .insert("ops".to_owned(), later + Duration::from_secs(9));
        assert!(
            matches!(reader.next(later), Job::Fleet),
            "a slow read starves nothing"
        );
        reader.fleet_at = Some(later);
        assert_eq!(viewing(&reader.next(later)), Some("api"));
        reader.read_at.insert("api".to_owned(), later);
        assert!(
            matches!(reader.next(later), Job::Wait(_)),
            "a neighbour read after the last focus waits"
        );
        reader.focus_at = later;
        assert!(
            matches!(reader.next(later), Job::View(ref name) if name == "far" || name == "web" || name == "end")
        );
        for name in ["far", "web", "end"] {
            reader.read_at.insert(name.to_owned(), later);
        }
        let much_later = later + REFRESH * 10;
        reader.fleet_at = Some(much_later);
        for name in ["ops", "api"] {
            reader.read_at.insert(name.to_owned(), much_later);
        }
        assert!(
            matches!(reader.next(much_later), Job::Wait(_)),
            "the rest is read once; a neighbour once per focus"
        );
    }

    /// I3: a foreign console bound to an identity its session no longer
    /// records is dropped, and so is its read stamp, so it reads afresh.
    #[test]
    fn a_console_bound_to_a_replaced_identity_is_dropped() {
        let root = Root::new("evict");
        let dir = session(&root, "web", "");
        let mut reader = Reader::new(root.0.clone(), None, None, None);
        reader.rows = rows(&["web"]);
        reader.dirs.insert("web".to_owned(), dir);
        reader.ids.insert("web".to_owned(), ID.to_owned());
        assert!(matches!(reader.view("web"), Some(Answer::View(view)) if view.id == ID));
        reader.evict();
        assert!(
            reader.consoles.contains_key("web"),
            "the same identity stays"
        );
        let other = "0199c0de-0000-4890-abcd-ef0123456789";
        reader.ids.insert("web".to_owned(), other.to_owned());
        reader.evict();
        assert!(!reader.consoles.contains_key("web"));
        assert!(!reader.read_at.contains_key("web"), "due again");
    }

    /// A redrawn lane keeps the coverage that still stands: the board's
    /// follow reports a gap once (the chat prints it once), and every later
    /// read of the same session, home or foreign, still shows it.
    #[test]
    fn standing_coverage_survives_every_later_read() {
        let root = Root::new("coverage");
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None, None);
        let _ = reader.open_home();
        let first = coverage(reader.view("api"));
        assert!(
            first.iter().any(|row| row.contains("api:lead")),
            "a seat with no conversation is a gap: {first:?}"
        );
        assert_eq!(
            coverage(reader.view("api")),
            first,
            "the home gap still stands"
        );
        let mut foreign = Reader::new(root.0.clone(), None, None, None);
        foreign
            .dirs
            .insert("api".to_owned(), root.0.join("sessions").join("api"));
        assert_eq!(coverage(foreign.view("api")), first);
        assert_eq!(
            coverage(foreign.view("api")),
            first,
            "a foreign view keeps it too"
        );
    }

    /// #19: each viewed foreign session reads through its own console.
    #[test]
    fn each_viewed_session_reads_through_its_own_console() {
        let root = Root::new("cache");
        let mut reader = Reader::new(root.0.clone(), None, None, None);
        for name in ["web", "ops"] {
            reader
                .dirs
                .insert(name.to_owned(), session(&root, name, ""));
        }
        for (viewed, other) in [("web", "ops"), ("ops", "web"), ("web", "ops")] {
            let coverage = coverage(reader.view(viewed)).join("\n");
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

    /// #9-13, #37-39: the fold over one read. Seat facts for every session
    /// still running, a last sign of life for every stopped one (its own
    /// launch row), needs only for a running session that asks for
    /// attention, each session's recorded identity, and the home pair.
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
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None, None);
        let _ = reader.open_home();
        let read = reader.fold(dirs, world, None, &FleetOrder::EMPTY, Timestamp::now());
        let keys = |map: Vec<&String>| map.into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(keys(read.facts.keys().collect()), ["run", "unk"]);
        assert_eq!(keys(read.needs.keys().collect()), ["run"]);
        let stopped = read.fleet.rows.iter().find(|row| row.name == "stop");
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
        let rows = &read.needs["run"].rows;
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
        assert!(read.ids.values().all(|id| id == ID), "{:?}", read.ids);
        assert_eq!(
            read.pair,
            Some(vec!["lead".to_owned(), "colead".to_owned()])
        );
    }

    /// I5: the kept draft rides only on a reading that can promote — owner
    /// after not-owner, as the composer counts (unknown changes nothing).
    #[test]
    fn only_a_reading_that_can_promote_carries_the_kept_draft() {
        let root = Root::new("carry");
        crate::store::open(&root.0.join("sessions").join("api"))
            .publish_console_draft(b"kept")
            .expect("a kept draft");
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None, None);
        let _ = reader.open_home();
        let away = || Reading::NotOwner("owned by window @2".to_owned());
        let readings = [
            Reading::Unknown,
            Reading::Owner,
            Reading::Owner,
            Reading::Unknown,
            Reading::Owner,
            away(),
            Reading::Owner,
        ];
        let carried: Vec<Option<Draft>> = readings
            .into_iter()
            .map(|reading| match reader.carry(reading, Instant::now()) {
                Answer::Owned { draft, .. } => draft,
                _ => panic!("an ownership answer"),
            })
            .collect();
        let kept = Some(Draft::Kept(b"kept".to_vec()));
        assert_eq!(carried, [None, kept.clone(), None, None, None, None, kept]);
    }
}
