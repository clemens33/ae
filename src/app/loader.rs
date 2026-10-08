//! The one background reader of `ae app`: every read of the world the app
//! draws — the fleet, the looks, whether home may be written, each session's
//! lane — happens on this thread, so a key never waits behind one. The UI thread
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
use crate::brief::{self, Filed};
use crate::console::input::Reading;
use crate::console::lane::{Lane, Seat};
use crate::console::needs::{SeatRef, Section};
use crate::console::{self, Console, term};
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
    /// The settings overlay opened (`true`) or closed, pinned to a generation.
    Settings { open: bool, generation: u64 },
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
    Fleet(FleetRead),
    /// Session `name`'s drawn look and the viewer's zone.
    Look {
        name: String,
        look: Option<theme::Look>,
        zone: Option<String>,
    },
    /// Whether home may be written, read at `at`.
    Owned {
        reading: Reading,
        at: Instant,
    },
    View(ViewRead),
    /// The settings overlay's bodies for `generation`. Quota rides every
    /// answer; config and about ride the first per open, then stay.
    Settings {
        generation: u64,
        quota: Vec<super::settings::QuotaRow>,
        config: Option<super::settings::ConfigView>,
        about: Option<super::settings::AboutFacts>,
    },
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
    /// Each session's memo, read and folded here: the show only ages it.
    pub(super) memos: BTreeMap<String, Result<Vec<Filed>, String>>,
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
    Settings,
    Wait(Instant),
}

/// The paths the settings reads resolve through, selected once by the UI.
#[derive(Debug, Clone, Default)]
pub(super) struct SettingsPaths {
    /// The operator's home, where default client config homes live.
    pub(super) home: Option<PathBuf>,
    /// The current global ae config.
    pub(super) global: PathBuf,
    /// The invocation-local ae config, when there is one.
    pub(super) local: Option<PathBuf>,
}

/// Everything the reader holds between reads.
pub(super) struct Reader {
    root: PathBuf,
    home: Option<String>,
    server: Option<ServerId>,
    home_console: Option<Console>,
    /// The home lead pair as it opened: home is writable while it stands.
    seats: Vec<Seat>,
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
    seq: u64,
    settings_paths: SettingsPaths,
    /// The open overlay's generation and its last quota read, if any.
    settings: Option<(u64, Option<Instant>)>,
    /// Config and about already read for the open overlay.
    settings_full: bool,
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
    pub(super) fn new(root: PathBuf, home: Option<String>, server: Option<ServerId>) -> Self {
        Self {
            root,
            home,
            server,
            home_console: None,
            seats: Vec::new(),
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
            seq: 0,
            settings_paths: SettingsPaths::default(),
            settings: None,
            settings_full: false,
        }
    }

    /// The paths the settings reads resolve through; set once before spawn.
    pub(super) fn settings_paths(&mut self, paths: SettingsPaths) -> &mut Self {
        self.settings_paths = paths;
        self
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
                Job::Settings => vec![self.settings()],
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
        self.seats = pair.clone().unwrap_or_default();
        self.home_console = Some(Console::open_standing(name, dir));
        Answer::Home(Some((writes, pair)))
    }

    pub(super) fn take(&mut self, request: Request) {
        match request {
            Request::Focus(selected) => {
                let home = self.home.as_ref();
                if let Some(name) = selected
                    .as_ref()
                    .filter(|name| !self.home_drawn && home != Some(*name))
                {
                    self.looks.insert(name.clone());
                }
                self.selected = selected;
                self.focus_at = Instant::now();
                self.evict();
            }
            Request::Reread(name) => drop(self.rereads.insert(name)),
            Request::Settings { open, generation } => {
                if open {
                    self.settings = Some((generation, None));
                    self.settings_full = false;
                } else {
                    self.settings = None;
                    self.settings_full = false;
                }
            }
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
    /// The open overlay's settings read immediately, then every refresh,
    /// below the looks so it never starves the main view.
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
        if let Some((_, last)) = self.settings {
            let due = last.map_or(now, |at| at + REFRESH);
            if due <= now {
                return Job::Settings;
            }
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
        let until = candidates
            .iter()
            .filter_map(|(_, due)| *due)
            .fold(fleet_at + REFRESH, Instant::min);
        // A first settings read never waits: it is due at once, above.
        let until = match self.settings {
            Some((_, Some(last))) => until.min(last + REFRESH),
            _ => until,
        };
        Job::Wait(until.min(now + REFRESH))
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
        if let Some(name) = self.undressed() {
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
        answers.push(Answer::Fleet(read));
        answers
    }

    /// Fold one read of the fleet: the seat facts of every live session, the
    /// stopped ones' last sign of life, the needs of every live session that
    /// asks for attention, each session's identity and memo, the sidebar rows
    /// and the home pair.
    pub(super) fn fold(
        &self,
        dirs: BTreeMap<String, PathBuf>,
        mut world: World,
        picker: Option<&[tmux::PickerSession]>,
        order: &theme::FleetOrder,
        now: Timestamp,
    ) -> FleetRead {
        // A tmux client's touch is a human's too: the later of it and the ask.
        let touches: BTreeMap<&str, i64> = picker
            .into_iter()
            .flatten()
            .filter_map(|row| Some((row.name.as_str(), row.touched()?)))
            .collect();
        for entry in &mut world.sessions {
            if let Some(at) = touches.get(entry.name.as_str()) {
                entry.human_epoch = entry.human_epoch.max(Some(*at));
            }
        }
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
        let home = self.home.as_deref();
        let fleet = fleet::rows(&world, &facts, &last_live, order, &needs, home, now);
        let ids = dirs
            .iter()
            .map(|(name, dir)| (name.clone(), console::recorded_uuid(dir)))
            .collect();
        let memos = dirs
            .iter()
            .map(|(name, dir)| {
                let memo = store::open(dir)
                    .memo_bytes()
                    .map(|bytes| brief::filed(&bytes))
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

    /// The selection whose look a fleet read reads beside home's: one while
    /// home's is not drawn, and never home's twice.
    fn undressed(&self) -> Option<String> {
        self.selected
            .clone()
            .filter(|name| !self.home_drawn && self.home.as_ref() != Some(name))
    }

    /// Session `name`'s drawn look, and whether home's is drawn.
    pub(super) fn look(&mut self, name: &str) -> Answer {
        let (look, zone) = console::look_of(name, true);
        self.noted(name, look.is_some());
        Answer::Look {
            name: name.to_owned(),
            look,
            zone,
        }
    }

    /// Session `name`'s look is read, `drawn` or not: home's says whether
    /// home's is drawn.
    fn noted(&mut self, name: &str, drawn: bool) {
        self.looks.remove(name);
        if self.home.as_deref() == Some(name) {
            self.home_drawn = drawn;
        }
    }

    /// Whether home may be written, read now.
    pub(super) fn owned(&self) -> Option<Answer> {
        let reading = reading(self.home_console.as_ref()?, &self.seats);
        Some(Answer::Owned {
            reading,
            at: Instant::now(),
        })
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
        Some(Answer::View(ViewRead {
            name: name.to_owned(),
            id,
            seq: self.seq,
            lane,
            needs,
            roster,
        }))
    }

    /// The open overlay's bodies for its generation: quota every time,
    /// config and about once per open. The read gate holds the whole read
    /// before anything is touched.
    fn settings(&mut self) -> Answer {
        crate::read_gate("settings");
        let (generation, _) = self.settings.unwrap_or((0, None));
        let paths = self.settings_paths.clone();
        let quota = {
            let roots = crate::inventory::Roots::under(&self.root);
            let inputs = crate::quota::Inputs {
                home: paths.home.as_deref(),
                global: Some(paths.global.as_path()),
                local: paths.local.as_deref(),
                sessions: Some(roots.sessions()),
                now: crate::time::Timestamp::now().epoch(),
            };
            crate::quota::quota_dialog_rows(&inputs)
                .into_iter()
                .map(|row| super::settings::QuotaRow {
                    label: row.label,
                    header: matches!(row.kind, crate::quota::DialogRowKind::Header),
                })
                .collect()
        };
        let (config, about) = if self.settings_full {
            (None, None)
        } else {
            let home_dir = self
                .home
                .as_deref()
                .and_then(|home| crate::console::locate(&self.root, home));
            let config = super::settings::resolve_config(home_dir.as_deref(), &paths.global);
            let about_dir = home_dir.unwrap_or_else(|| self.root.join("sessions"));
            (Some(config), Some(self.about(&about_dir, &paths.global)))
        };
        self.settings = Some((generation, Some(Instant::now())));
        self.settings_full = true;
        Answer::Settings {
            generation,
            quota,
            config,
            about,
        }
    }

    /// The about tab's read facts for the home session at `home_dir`.
    fn about(
        &self,
        home_dir: &std::path::Path,
        current_global: &std::path::Path,
    ) -> super::settings::AboutFacts {
        let (tmux_version, tmux_verdict) = match self.server.as_ref() {
            Some(server) => {
                let probe = crate::transport::observe_tmux_floor(server);
                (probe.found().to_owned(), probe.verdict().as_str())
            }
            None => (String::new(), "unknown"),
        };
        let server = match crate::session_launch::recorded_server_resolved(home_dir) {
            Some(crate::inventory::ServerId::Ambient) => "ambient".to_owned(),
            Some(selected) => crate::tmux::server_args(&selected).join(" "),
            None => "unresolvable".to_owned(),
        };
        super::settings::AboutFacts {
            tmux_version,
            tmux_verdict,
            state_root: self.root.display().to_string(),
            config_file: current_global.display().to_string(),
            server,
        }
    }
}

/// Whether `console`'s session may be written, as the composer takes it:
/// its incarnation and lead pair are still the `seats` it opened with.
pub(super) fn reading(console: &Console, seats: &[Seat]) -> Reading {
    match term::still(seats, console.seats()) {
        Ok(()) => Reading::Owner,
        Err(why) => Reading::NotOwner(why),
    }
}

#[cfg(test)]
mod tests {
    //! Oracles: the brief's rulings 2 and 3 (priority, cadence), ruling
    //! appuse-b R-B5/R-B7 (what keeps home writable), the fixture root's own
    //! records and the existing owners' coverage words.

    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::{Answer, Job, KEEP, Reader, Request, Tier, order};
    use crate::app::REFRESH;
    use crate::app::fleet::Line2;
    use crate::app::tests::{ID, Root, entry, meta, session};
    use crate::attention::Reason;
    use crate::console::input::Reading;
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
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
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

    /// A selection with no console to read through is never read: the
    /// reader goes on to the rest instead of asking for it again and again.
    #[test]
    fn an_unreadable_selection_is_never_read() {
        let root = Root::new("ghost");
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
        let now = Instant::now();
        reader.fleet_at = Some(now);
        reader.rows = rows(&["ghost", "api", "web"]);
        reader.dirs.insert("web".to_owned(), PathBuf::from("web"));
        reader.take(Request::Focus(Some("ghost".to_owned())));
        reader.looks.clear();
        assert_eq!(
            viewing(&reader.next(now)),
            Some("web"),
            "neither the dir-less selection nor an unopened home"
        );
    }

    /// Ruling 3: a neighbour read before the last change of selection is read
    /// again a REFRESH after that read completed, not sooner.
    #[test]
    fn a_stale_neighbour_waits_out_its_refresh() {
        let root = Root::new("warm");
        let mut reader = Reader::new(root.0.clone(), None, None);
        let read = Instant::now();
        reader.rows = rows(&["a", "b"]);
        reader.take(Request::Focus(Some("a".to_owned())));
        reader.looks.clear();
        for name in ["a", "b"] {
            reader.dirs.insert(name.to_owned(), PathBuf::from(name));
            reader.read_at.insert(name.to_owned(), read);
        }
        let now = read + REFRESH / 2;
        reader.focus_at = now;
        reader.fleet_at = Some(now);
        assert!(matches!(reader.next(now), Job::Wait(until) if until == read + REFRESH));
        reader.read_at.insert("a".to_owned(), now);
        assert_eq!(viewing(&reader.next(read + REFRESH)), Some("b"));
    }

    /// A selection's look is read only while home's is not drawn, and
    /// home's never twice: a focus queues it, a fleet read reads it.
    #[test]
    fn looks_are_read_for_home_and_an_undressed_selection_only() {
        let root = Root::new("looks");
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
        reader.take(Request::Focus(Some("api".to_owned())));
        assert!(
            reader.looks.is_empty(),
            "home's look rides every fleet read"
        );
        assert_eq!(reader.undressed(), None, "home's look once");
        reader.take(Request::Focus(Some("web".to_owned())));
        assert!(reader.looks.contains("web"));
        assert_eq!(reader.undressed().as_deref(), Some("web"));
        reader.home_drawn = true;
        assert_eq!(reader.undressed(), None, "a drawn home dresses the app");
        reader.looks.clear();
        reader.take(Request::Focus(Some("ops".to_owned())));
        assert!(reader.looks.is_empty(), "and queues no look");
    }

    /// Only home's own look says whether home's look is drawn.
    #[test]
    fn only_homes_look_says_home_is_drawn() {
        let root = Root::new("drawn");
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
        reader.looks.insert("web".to_owned());
        reader.noted("web", true);
        assert!(!reader.home_drawn);
        assert!(reader.looks.is_empty(), "a read look is no longer asked");
        reader.noted("api", true);
        assert!(reader.home_drawn);
        reader.noted("web", false);
        assert!(reader.home_drawn, "a foreign look leaves it");
        reader.noted("api", false);
        assert!(!reader.home_drawn);
    }

    /// I3: a foreign console bound to an identity its session no longer
    /// records is dropped, and so is its read stamp, so it reads afresh.
    #[test]
    fn a_console_bound_to_a_replaced_identity_is_dropped() {
        let root = Root::new("evict");
        let dir = session(&root, "web", "");
        let mut reader = Reader::new(root.0.clone(), None, None);
        reader.rows = rows(&["web"]);
        reader.dirs.insert("web".to_owned(), dir);
        reader.ids.insert("web".to_owned(), ID.to_owned());
        assert!(matches!(reader.view("web"), Some(Answer::View(view)) if view.id == ID));
        reader.read_at.insert("api".to_owned(), Instant::now());
        reader.home = Some("api".to_owned());
        reader.evict();
        assert!(
            reader.consoles.contains_key("web"),
            "the same identity stays"
        );
        assert!(
            reader.read_at.contains_key("web") && reader.read_at.contains_key("api"),
            "a kept console and home keep their read stamps"
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
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
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
        let mut foreign = Reader::new(root.0.clone(), None, None);
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
        let mut reader = Reader::new(root.0.clone(), None, None);
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
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
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

    /// R-B5/R-B7: home is writable while the incarnation and lead pair it
    /// opened with stand, re-read each time: a replaced session, a changed
    /// pair or a meta that is gone is not.
    #[test]
    fn home_is_writable_only_while_its_opened_incarnation_and_pair_stand() {
        let root = Root::new("owned");
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
        let _ = reader.open_home();
        let read = |reader: &Reader| match reader.owned() {
            Some(Answer::Owned { reading, .. }) => reading,
            _ => panic!("an ownership answer"),
        };
        assert_eq!(read(&reader), Reading::Owner);
        for (id, peer) in [
            (ID.replace("1234", "bbbb"), "colead"),
            (ID.to_owned(), "peer"),
        ] {
            meta(&root, &id, peer);
            assert!(matches!(read(&reader), Reading::NotOwner(_)));
        }
        std::fs::remove_file(root.0.join("sessions").join("api").join("meta")).expect("gone");
        assert!(matches!(read(&reader), Reading::NotOwner(_)));
        meta(&root, ID, "colead");
        assert_eq!(read(&reader), Reading::Owner);
    }

    /// I11: a settings read completed at t cannot reread at t or
    /// t+REFRESH/2; the reader waits until exactly t+REFRESH, then the
    /// settings read is due again. An open overlay reads immediately; a
    /// close stops the settings job.
    #[test]
    fn a_completed_settings_read_waits_a_full_refresh() {
        let root = Root::new("settings-due");
        let mut reader = Reader::new(root.0.clone(), None, None);
        let t = Instant::now();
        reader.fleet_at = Some(t);
        reader.take(Request::Settings {
            open: true,
            generation: 7,
        });
        assert!(
            matches!(reader.next(t), Job::Settings),
            "a first settings read never waits"
        );
        reader.settings = Some((7, Some(t)));
        assert!(
            matches!(reader.next(t), Job::Wait(until) if until == t + REFRESH),
            "a fresh completion waits the full refresh"
        );
        let half = t + REFRESH / 2;
        assert!(
            matches!(reader.next(half), Job::Wait(until) if until == t + REFRESH),
            "half a refresh later the deadline has not moved"
        );
        assert!(
            matches!(reader.next(t + REFRESH), Job::Settings),
            "at the deadline the settings read is due"
        );
        reader.take(Request::Settings {
            open: false,
            generation: 7,
        });
        assert!(
            matches!(reader.next(t + REFRESH), Job::Fleet),
            "a close stops the settings job"
        );
    }

    /// I12: the wait shortens to the settings deadline when a completed
    /// settings read predates the fleet read; at that deadline the
    /// settings read is due, and after a close the fleet deadline
    /// remains.
    #[test]
    fn the_wait_shortens_to_an_earlier_settings_deadline() {
        let root = Root::new("settings-wait");
        let mut reader = Reader::new(root.0.clone(), None, None);
        let t = Instant::now();
        let last = t
            .checked_sub(REFRESH / 2)
            .expect("a representable time before now");
        reader.fleet_at = Some(t);
        reader.settings = Some((7, Some(last)));
        assert!(
            matches!(reader.next(t), Job::Wait(until) if until == last + REFRESH),
            "the wait ends at the earlier settings deadline, not the fleet one"
        );
        assert!(
            matches!(reader.next(last + REFRESH), Job::Settings),
            "at the settings deadline the read is due"
        );
        reader.take(Request::Settings {
            open: false,
            generation: 7,
        });
        assert!(
            matches!(reader.next(t), Job::Wait(until) if until == t + REFRESH),
            "after a close the fleet deadline remains"
        );
    }
}
