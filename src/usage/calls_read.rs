//! The tool-statistics reader: association discovery and stepped, validated
//! transcript reads at the usage door (plan-r3 §5, §6).
//!
//! One [`Reader`] per app process, owned by the loader thread. A round lists
//! the ae-associated conversations (live seats and their priors, `retire`
//! records, kept archives' `harness.tsv`), locates each one's Claude or Codex
//! files, and reads them in units: one [`Reader::step`] does ONE unit of
//! bounded IO and returns what the doors did ([`Unit`]). Per-file state (dev,
//! ino, len, mtime, ctime, cursor, skipping, SHA-256 over the consumed prefix)
//! decides between no read, a verified resume and a reread from 0. Paths live
//! only in that per-file state, never in a [`Conversation`] or a [`View`].
//!
//! Every IO goes through an existing usage or quota door; the one door this
//! reader added is [`super::names_only`]. A [`Unit`]'s counters are the
//! actual receipt of its door calls ([`super::receipt`]), read through one
//! scoped guard per unit on the loader thread.
#![allow(
    dead_code,
    clippy::unused_self,
    reason = "B0 seam: the loader wires the reader in B4"
)]

use super::lines::{Framed, LINE_CAP, Lines};
use super::receipt::{Counts, Guard};
use super::{Entry, claude_calls, codex_calls};
use crate::meta::{RecordedClient, RecordedConfigHome, RecordedWorkDir, RosterEntry};
use crate::quota::{Bounded, Budget, DayListings};
use crate::tool::{ToolKind, UsageSource};
use crate::tool_stats::{
    Adapter, Ask, Caps, Conversation, Event, GapKind, GapLine, Membership, Parsed, Role, Scope,
    Stream, StreamRole, VIEW_RESERVE, View, Weight, event_floor, prefix, weigh,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The most bytes one unit reads.
pub(crate) const STEP: u64 = 4 << 20;
/// The most lstats one unit makes.
pub(crate) const LSTATS: u32 = 256;
/// A shown page's discovery round period.
pub(crate) const TOOLS_ROUND: Duration = Duration::from_secs(30);
/// An archive's `harness.tsv` cap (the archive module's own byte cap).
const HARNESS_MAX: u64 = 1 << 20;
/// The longest membership field (session id, slot, client label) the reader
/// retains. Session metas are hostile input: a membership with a longer field
/// is not kept and counts in a visible gap, so each kept membership costs at
/// most a fixed size on the ledger (B3 review r4 B3-1a).
const FIELD_MAX: usize = 256;
/// The most one kept membership owns: its struct and its three strings
/// (session id, slot, client label), each at most [`FIELD_MAX`] bytes.
const MEMBERSHIP_MAX: usize = size_of::<Membership>() + 3 * FIELD_MAX;

/// What one unit was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnitKind {
    /// A session meta read.
    Meta,
    /// One journal chunk of a session's retire/reseat scan.
    Journal,
    /// An archive's `harness.tsv`.
    Archive,
    /// One directory listing (`projects/`, `subagents/`, a Codex day).
    Listing,
    /// A batch of lstats.
    Lstats,
    /// One chunk re-hashing a consumed prefix.
    Verify,
    /// One chunk of new transcript bytes.
    Read,
    /// A persisted document loaded from `stats/tools/`.
    Load,
    /// One conversation's publication to `stats/tools/`.
    Publish,
    /// A recovery step of an interrupted publication.
    Recover,
}

/// What the doors did during one unit, counted AT the door operations:
/// bytes the reads returned, lstat/fstat calls, directory listings, opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unit {
    pub(crate) kind: UnitKind,
    /// Transcript and journal bytes read.
    pub(crate) bytes: u64,
    /// Bytes of `stats/tools/` read or written.
    pub(crate) store_bytes: u64,
    pub(crate) lstats: u32,
    pub(crate) listings: u32,
    pub(crate) opens: u32,
}

impl Unit {
    fn of(kind: UnitKind, counts: Counts) -> Self {
        let narrow = |count: u64| u32::try_from(count).unwrap_or(u32::MAX);
        Self {
            kind,
            bytes: counts.bytes,
            store_bytes: 0,
            lstats: narrow(counts.stats),
            listings: narrow(counts.listings),
            opens: narrow(counts.opens),
        }
    }
}

/// The stats this unit's doors made so far.
fn stats(guard: &Guard) -> u64 {
    guard.delta().stats
}

type Key = (Adapter, String);

/// Where a located file's observations go.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Place {
    conversation: Key,
    role: StreamRole,
    copy: u32,
}

/// A file's identity as one status reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Identity {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl Identity {
    fn of(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            len: metadata.len(),
            mtime: (metadata.mtime(), metadata.mtime_nsec()),
            ctime: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }

    const fn window(window: &super::JournalWindow) -> Self {
        Self {
            dev: window.dev,
            ino: window.ino,
            len: window.len,
            mtime: window.mtime,
            ctime: window.ctime,
        }
    }
}

/// The seat roots a Claude conversation's skill directories classify by.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Roots {
    config_homes: Vec<PathBuf>,
    work_dirs: Vec<PathBuf>,
}

/// One located file's read state (plan §6). The path lives only here.
struct Tracked {
    path: PathBuf,
    identity: Identity,
    lines: Lines,
    hash: ring::digest::Context,
    roots: Roots,
    /// The last read reached the end of the file as its status then said:
    /// only then may an unchanged status skip the file.
    done: bool,
}

impl Tracked {
    fn new(path: PathBuf, roots: Roots) -> Self {
        Self {
            path,
            identity: Identity::default(),
            lines: Lines::new(LINE_CAP),
            hash: ring::digest::Context::new(&ring::digest::SHA256),
            roots,
            done: false,
        }
    }
}

/// One conversation's association evidence, gathered over a round.
#[derive(Debug, Default)]
struct Assoc {
    memberships: Vec<Membership>,
    /// The session directories and archives whose evidence it rests on.
    sources: BTreeSet<String>,
    /// Each store holding a copy, in copy order.
    stores: BTreeSet<PathBuf>,
    roots: Roots,
}

/// Unsupported seats' conversations (B3 review r1 B3-5, B3-6): per
/// (normalized tool, conversation id), the sessions holding a membership of
/// it. One conversation counts once per tool however many memberships list
/// it, and only in the scopes its memberships reach.
type Unsupported = BTreeMap<(String, String), BTreeSet<Option<String>>>;

/// One unit of work.
enum Task {
    Sessions,
    Meta {
        dir: PathBuf,
        name: String,
    },
    Journal {
        dir: PathBuf,
        name: String,
        session: Option<String>,
        lines: Lines,
    },
    Archives,
    Archive {
        dir: PathBuf,
        id: String,
    },
    ClaudeStore {
        store: PathBuf,
    },
    ClaudeProject {
        store: PathBuf,
        project: PathBuf,
    },
    Subagents {
        store: PathBuf,
        id: String,
        dir: PathBuf,
    },
    CodexDay {
        dir: PathBuf,
    },
    CodexFind {
        root: PathBuf,
        id: String,
    },
    Lstats(Vec<(Place, PathBuf)>),
    /// `anchor` is the status every chunk of this pass must still see (B3
    /// review r1 B3-3): the lstat's that queued the pass, then each chunk's.
    Verify {
        place: Place,
        at: u64,
        hash: Box<ring::digest::Context>,
        anchor: Identity,
    },
    Read {
        place: Place,
        anchor: Option<Identity>,
    },
}

impl Task {
    const fn kind(&self) -> UnitKind {
        match self {
            Self::Meta { .. } => UnitKind::Meta,
            Self::Journal { .. } => UnitKind::Journal,
            Self::Archive { .. } => UnitKind::Archive,
            Self::Sessions
            | Self::Archives
            | Self::ClaudeStore { .. }
            | Self::ClaudeProject { .. }
            | Self::Subagents { .. }
            | Self::CodexDay { .. } => UnitKind::Listing,
            Self::CodexFind { .. } | Self::Lstats(_) => UnitKind::Lstats,
            Self::Verify { .. } => UnitKind::Verify,
            Self::Read { .. } => UnitKind::Read,
        }
    }
}

/// Where a round is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Associate,
    Locate,
    Check,
}

/// One round's discovery state.
struct Round {
    phase: Phase,
    tasks: VecDeque<Task>,
    assoc: BTreeMap<Key, Assoc>,
    /// Per session directory or archive: was its evidence complete.
    complete: BTreeMap<String, bool>,
    /// The archive listing itself was read.
    archives_listed: bool,
    reseated: BTreeSet<String>,
    /// Bytes of discovery state this round retains, the task queue's live
    /// allocation included, charged on the ledger beside the conversations
    /// (plan §7).
    bytes: usize,
    limit: usize,
    unsupported: Unsupported,
    /// Memberships not kept because a field passed [`FIELD_MAX`].
    oversize: usize,
    /// Claude: (store, id) → every project file `<id>.jsonl` found.
    parents: BTreeMap<(PathBuf, String), Vec<PathBuf>>,
    /// Claude: (store, id) → its `subagents/` files, by stem.
    children: BTreeMap<(PathBuf, String), BTreeMap<String, PathBuf>>,
    /// Stores or roots whose listing failed or was cut.
    broken: BTreeSet<PathBuf>,
    days: DayListings,
    /// Day directories whose listing (or proven absence) is cached.
    listed_days: BTreeSet<PathBuf>,
    /// Codex: (root, id) → the rollout, or why not.
    rollouts: BTreeMap<(PathBuf, String), Result<Option<PathBuf>, GapKind>>,
    /// Resolves an implicit harness store.
    home: Option<PathBuf>,
    /// What the admitted conversations and their files hold, and what the
    /// refused ones keep (their records and refusal entries), refreshed
    /// before every unit and every queued task: discovery shares the ledger
    /// with them (B3 review r2 B3-1a, r3 B3-1a). Association and location
    /// may displace what is held (it is a cache, read again when room
    /// frees, B3 review r3 B3-7); association also what is pinned, as it
    /// rebuilds every record (lead ruling ae-20261108T170544Z-0d20805a);
    /// checking may not.
    held: usize,
    pinned: usize,
}

impl Round {
    fn new(limit: usize, home: Option<PathBuf>) -> Self {
        let tasks = VecDeque::from([Task::Sessions, Task::Archives]);
        Self {
            phase: Phase::Associate,
            bytes: tasks.capacity() * SLOT + tasks.iter().map(task_bytes).sum::<usize>(),
            tasks,
            assoc: BTreeMap::new(),
            complete: BTreeMap::new(),
            archives_listed: true,
            reseated: BTreeSet::new(),
            limit,
            unsupported: Unsupported::new(),
            oversize: 0,
            parents: BTreeMap::new(),
            children: BTreeMap::new(),
            broken: BTreeSet::new(),
            days: DayListings::default(),
            listed_days: BTreeSet::new(),
            rollouts: BTreeMap::new(),
            home,
            held: 0,
            pinned: 0,
        }
    }
}

/// The reader's whole state.
pub(crate) struct Reader {
    state_root: PathBuf,
    home: Option<PathBuf>,
    caps: Caps,
    /// Sorted by `(adapter, id)`: the fold's input.
    conversations: Vec<Conversation>,
    files: BTreeMap<Place, Tracked>,
    /// Files reset to 0 this round and not yet read again.
    recounting: BTreeSet<Place>,
    /// Unsupported seats from the last association.
    unsupported: Unsupported,
    /// Association sources whose evidence was unavailable last round.
    hidden: usize,
    /// Memberships last round's association did not keep because a field
    /// passed [`FIELD_MAX`].
    oversize: usize,
    /// Each conversation's ledger weight, aligned with `conversations`;
    /// recomputed only when that conversation's evidence changes.
    weights: Vec<Weight>,
    /// Conversations the reader's ledger refused this round: their evidence
    /// and file state are evicted (plan §7, B3 review r1 B3-1). One refused
    /// by the prefix rule is read again once the room before it grew (B3
    /// review r2 B3-7); one refused alone waits for the next round.
    refused: BTreeMap<Key, Refusal>,
    /// The conversation a readmission is reading again, until the queue
    /// drains.
    retrying: Option<usize>,
    /// Times something held shrank (a recount, an alone refusal): a
    /// conversation refused during its own readmission waits for the next.
    shrinks: u64,
    round: Option<Round>,
}

/// Why the ledger refused a conversation this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// Over the observation or turn cap, whatever the room.
    Alone,
    /// By the prefix rule, when `room` bytes of the ledger were left before
    /// it (0 when an earlier conversation was refused already); `settled`
    /// is the shrink count when it was refused during its own readmission.
    Prefix { room: usize, settled: Option<u64> },
}

impl std::fmt::Debug for Reader {
    /// Counts only: the per-file state holds paths, which never leave it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader")
            .field("conversations", &self.conversations.len())
            .field("files", &self.files.len())
            .field("busy", &self.busy())
            .finish_non_exhaustive()
    }
}

impl Reader {
    /// A reader of the ae state root `state_root`; `home` resolves an implicit
    /// harness store. Nothing is read until [`Self::round`].
    pub(crate) const fn new(state_root: PathBuf, home: Option<PathBuf>, caps: Caps) -> Self {
        Self {
            state_root,
            home,
            caps,
            conversations: Vec::new(),
            files: BTreeMap::new(),
            recounting: BTreeSet::new(),
            unsupported: Unsupported::new(),
            hidden: 0,
            oversize: 0,
            weights: Vec::new(),
            refused: BTreeMap::new(),
            retrying: None,
            shrinks: 0,
            round: None,
        }
    }

    /// Start a discovery round (on show, then every [`TOOLS_ROUND`]); a round
    /// in progress continues.
    pub(crate) fn round(&mut self) {
        if self.round.is_none() {
            self.round = Some(Round::new(
                self.caps.ledger.saturating_sub(VIEW_RESERVE),
                self.home.clone(),
            ));
        }
    }

    /// Do ONE unit of the round; `None` when the round is done.
    pub(crate) fn step(&mut self) -> Option<Unit> {
        loop {
            let now = self.holding();
            let round = self.round.as_mut()?;
            let Some(task) = round.pop() else {
                if !self.advance() {
                    self.round = None;
                    return None;
                }
                continue;
            };
            // A queued task is discovery state until it runs; what is held
            // and what the refused keep share the ledger with what the unit
            // may keep (B3 review r2 B3-1a, r3 B3-1a).
            round.held = now.admitted;
            round.pinned = now.pinned;
            // The unit's receipt is what the doors actually did on this
            // thread between its start and its end (lead ruling
            // ae-20261108T141228Z-383a41ad, B3 review r1 B3-2).
            let kind = task.kind();
            let guard = super::receipt::Guard::start();
            self.run(task, &guard);
            let unit = Unit::of(kind, guard.delta());
            // Discovery the unit kept may have displaced held observations:
            // the ledger decides again before the next unit (B3 review r3
            // B3-7).
            self.enforce();
            observed(unit);
            return Some(unit);
        }
    }

    /// Whether a round is in progress.
    pub(crate) const fn busy(&self) -> bool {
        self.round.is_some()
    }

    /// The page `ask` over everything admitted so far, with the unsupported
    /// seats' coverage lines and the recount count.
    pub(crate) fn view(&self, ask: &Ask) -> View {
        let mut view = crate::tool_stats::fold(&self.conversations, ask, &self.caps);
        let mut unsupported: BTreeMap<&str, u64> = BTreeMap::new();
        for ((tool, _), sessions) in &self.unsupported {
            let shown = match &ask.scope {
                Scope::All => true,
                Scope::This(None) => false,
                Scope::This(Some(session)) => sessions
                    .iter()
                    .any(|held| held.as_deref() == Some(session.as_str())),
            };
            if shown {
                *unsupported.entry(tool).or_default() += 1;
            }
        }
        for (tool, count) in unsupported {
            view.gaps.push(GapLine {
                kind: GapKind::Unsupported(tool.to_owned()),
                conversation: None,
                count,
            });
        }
        if self.hidden > 0 {
            view.gaps.push(GapLine {
                kind: GapKind::Unreadable,
                conversation: None,
                count: u64::try_from(self.hidden).unwrap_or(u64::MAX),
            });
        }
        if self.oversize > 0 {
            view.gaps.push(GapLine {
                kind: GapKind::FieldOverLimit,
                conversation: None,
                count: u64::try_from(self.oversize).unwrap_or(u64::MAX),
            });
        }
        view.gaps.sort();
        view.recounting = u64::try_from(self.recounting.len()).unwrap_or(u64::MAX);
        view
    }

    /// The retained observations (the privacy surface besides the view).
    pub(crate) fn conversations(&self) -> &[Conversation] {
        &self.conversations
    }

    fn run(&mut self, task: Task, guard: &Guard) {
        match task {
            Task::Sessions => self.sessions(),
            Task::Meta { dir, name } => self.meta(&dir, name),
            Task::Journal {
                dir,
                name,
                session,
                lines,
            } => self.journal(dir, name, session, lines),
            Task::Archives => self.archives(),
            Task::Archive { dir, id } => self.archive(&dir, &id),
            Task::ClaudeStore { store } => self.claude_store(store),
            Task::ClaudeProject { store, project } => self.claude_project(store, &project),
            Task::Subagents { store, id, dir } => self.subagents(store, id, &dir),
            Task::CodexDay { dir } => self.codex_day(&dir),
            Task::CodexFind { root, id } => self.codex_find(root, id),
            Task::Lstats(batch) => self.lstats(batch, guard),
            Task::Verify {
                place,
                at,
                hash,
                anchor,
            } => self.verify(place, at, hash, anchor),
            Task::Read { place, anchor } => self.read(place, anchor),
        }
    }

    /// Move to the next phase when the queue ran dry; `false` when the round
    /// is over.
    fn advance(&mut self) -> bool {
        let Some(round) = self.round.as_mut() else {
            return false;
        };
        match round.phase {
            Phase::Associate => {
                round.phase = Phase::Locate;
                self.associate();
                true
            }
            Phase::Locate => {
                // The lstats are location's last discovery: queued before
                // checking starts, so they may displace held observations.
                self.place();
                if let Some(round) = self.round.as_mut() {
                    round.phase = Phase::Check;
                }
                true
            }
            // Drained: a conversation the prefix refused earlier this round
            // may fit now that discovery was released or earlier ones shrank
            // (B3 review r2 B3-7, r3 B3-7).
            Phase::Check => {
                self.retrying = None;
                self.readmit()
            }
        }
    }
}

// --- Association (plan §5) -------------------------------------------------

impl Reader {
    fn sessions(&mut self) {
        let root = self.state_root.join("sessions");
        let Some(round) = self.round.as_mut() else {
            return;
        };
        match crate::quota::session_paths(&root, &mut Budget::local(u64::MAX)) {
            Ok(Bounded::Ready(paths)) => {
                for dir in paths {
                    let name = dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned);
                    if let Some(name) = name {
                        round.push(Task::Meta { dir, name });
                    }
                }
            }
            Ok(Bounded::Truncated) | Err(_) => round.mark("sessions/", false),
        }
    }

    fn meta(&mut self, dir: &Path, name: String) {
        let mut budget = Budget::local(u64::MAX);
        let bytes = super::meta_bytes(&dir.join(crate::meta::FILE), &mut budget);
        let Some(round) = self.round.as_mut() else {
            return;
        };
        let Some(text) = bytes.ok().and_then(|bytes| String::from_utf8(bytes).ok()) else {
            round.mark(&name, false);
            return;
        };
        let meta = crate::meta::Meta::parse(&text);
        let session = meta.session_id().map(str::to_owned);
        let session_dir = meta.work_dir().map(PathBuf::from);
        for entry in meta.roster() {
            let roots = roots_of(entry, session_dir.as_deref());
            let membership = |role: Role, client: RecordedClient| Membership {
                session: session.clone(),
                slot: entry.slot.clone(),
                role,
                client,
            };
            if let Some(id) = super::valid_id(entry.harness_session.as_deref()) {
                round.add(
                    entry,
                    &id,
                    &name,
                    membership(Role::Live, entry.client.clone()),
                    &roots,
                );
            }
            for raw in meta.harness_session_prior(&entry.slot) {
                let Some(prior) = crate::meta::prior_parts(raw) else {
                    continue;
                };
                let mut seat = entry.clone();
                if let Some(tool) = prior.tool
                    && Some(tool) != entry.binary.as_deref()
                {
                    // Another tool's conversation: its store is not this
                    // seat's, so it is listed but cannot be located.
                    seat.binary = Some(tool.to_owned());
                    seat.config_home = RecordedConfigHome::Invalid;
                }
                seat.harness_session = Some(prior.id.to_owned());
                if let Some(id) = super::valid_id(Some(prior.id)) {
                    round.add(
                        &seat,
                        &id,
                        &name,
                        membership(Role::Prior, RecordedClient::Missing),
                        &roots,
                    );
                }
            }
        }
        round.mark(&name, true);
        round.push(Task::Journal {
            dir: dir.to_owned(),
            name,
            session,
            lines: Lines::new(LINE_CAP),
        });
    }

    fn journal(&mut self, dir: PathBuf, name: String, session: Option<String>, mut lines: Lines) {
        let path = dir.join(crate::store::EVENTS);
        let Some(round) = self.round.as_mut() else {
            return;
        };
        // A missing journal is unavailable evidence, never an empty one (B3
        // review r1 B3-4): it could have held a retire or reseat.
        let Ok(window) = super::journal_window(&path, lines.cursor(), STEP) else {
            round.mark(&name, false);
            return;
        };
        let mut damaged = false;
        let mut records = Vec::new();
        let consumed = lines.step(&window.bytes, &mut |framed| match framed {
            Framed::OverCap => damaged = true,
            Framed::Line(line) => {
                if line.iter().all(u8::is_ascii_whitespace) {
                    return;
                }
                match std::str::from_utf8(line)
                    .ok()
                    .and_then(|text| crate::events::Event::parse_line(text).ok())
                {
                    Some(event) => records.push(event),
                    None => damaged = true,
                }
            }
        });
        for event in &records {
            round.record(event, &name, session.as_deref());
        }
        if damaged || lines.skipping() {
            round.mark(&name, false);
        }
        // Bytes past the cursor remain: read on while a step consumes; a step
        // that consumes nothing left only a tail with no LF, an unfinished
        // record, so the scan is not complete.
        if lines.cursor() < window.len {
            if consumed > 0 {
                round.push(Task::Journal {
                    dir,
                    name,
                    session,
                    lines,
                });
            } else {
                round.mark(&name, false);
            }
        }
    }

    fn archives(&mut self) {
        let root = self.state_root.clone();
        let Some(round) = self.round.as_mut() else {
            return;
        };
        let mut budget = Budget::local(u64::MAX);
        let dir = match super::derived_directory(&root, "archive", &mut budget) {
            Ok(Some(dir)) => dir,
            Ok(None) => return,
            Err(_) => {
                round.archives_listed = false;
                return;
            }
        };
        match super::names_only(&dir, &mut budget) {
            Ok(Bounded::Ready(names)) => {
                for (id, kind) in names {
                    if kind == Entry::Dir && crate::archive::is_canonical_uuid(&id) {
                        round.push(Task::Archive {
                            dir: dir.join(&id),
                            id,
                        });
                    }
                }
            }
            Ok(Bounded::Truncated) | Err(_) => round.archives_listed = false,
        }
    }

    fn archive(&mut self, dir: &Path, id: &str) {
        let Some(round) = self.round.as_mut() else {
            return;
        };
        let source = format!("archive/{id}");
        let path = dir.join(crate::archive::harness::FILE);
        // A kept archive without its harness record may hide memberships.
        let Ok(bytes) = super::bounded_bytes(&path, HARNESS_MAX, &mut Budget::local(u64::MAX))
        else {
            round.mark(&source, false);
            return;
        };
        let Ok(record) = crate::archive::harness::parse(&bytes, id) else {
            round.mark(&source, false);
            round.archives_listed = false;
            return;
        };
        round.mark(&source, true);
        for slot in &record.slots {
            let seat = RosterEntry {
                slot: slot.slot.clone(),
                name: String::new(),
                profile: None,
                client: RecordedClient::Missing,
                harness_session: None,
                config_home: slot.config_home.clone(),
                config_home_base: slot.config_home_base.clone(),
                binary: Some(slot.binary.clone()),
                work_dir: RecordedWorkDir::Missing,
            };
            let roots = roots_of(&seat, None);
            for conversation in slot.current.iter().chain(&slot.priors) {
                let mut seat = seat.clone();
                if conversation.tool != slot.binary {
                    seat.binary = Some(conversation.tool.clone());
                    seat.config_home = RecordedConfigHome::Invalid;
                }
                seat.harness_session = Some(conversation.id.clone());
                let membership = Membership {
                    session: Some(record.archive_id.clone()),
                    slot: slot.slot.clone(),
                    role: Role::Archived,
                    client: RecordedClient::Missing,
                };
                round.add(&seat, &conversation.id, &source, membership, &roots);
            }
        }
    }

    /// Association is done: publish memberships and evidence onto the
    /// conversations, drop the ones no longer associated, queue the locate
    /// listings.
    fn associate(&mut self) {
        let Some(round) = self.round.as_mut() else {
            return;
        };
        self.unsupported = std::mem::take(&mut round.unsupported);
        self.refused.clear();
        self.retrying = None;
        // Any source whose evidence is unavailable (a session meta, journal,
        // archive record, or a listing) may hide a membership of ANY
        // conversation, so no client is proven this round (plan §5 (a), B3
        // review r1 B3-4); the count is a path-free coverage line.
        self.hidden = round
            .complete
            .values()
            .filter(|complete| !**complete)
            .count()
            + usize::from(!round.archives_listed);
        // A membership not kept may be any conversation's: no client is
        // proven this round either.
        self.oversize = round.oversize;
        let hidden = self.hidden > 0 || self.oversize > 0;
        let complete = |sources: &BTreeSet<String>| {
            !hidden
                && sources
                    .iter()
                    .all(|source| round.complete.get(source).copied().unwrap_or(false))
        };
        let mut old: BTreeMap<Key, Conversation> = std::mem::take(&mut self.conversations)
            .into_iter()
            .map(|conversation| {
                (
                    (conversation.adapter, conversation.id.clone()),
                    conversation,
                )
            })
            .collect();
        let mut next = Vec::with_capacity(round.assoc.len());
        let mut moved = 0_usize;
        for (key, assoc) in &mut round.assoc {
            let mut conversation = old.remove(key).unwrap_or_else(|| Conversation {
                adapter: key.0,
                id: key.1.clone(),
                memberships: Vec::new(),
                evidence_complete: false,
                reseated: false,
                streams: Vec::new(),
                gaps: Vec::new(),
                read: false,
            });
            // One owner (lead ruling ae-20261108T170544Z-0d20805a): the
            // memberships MOVE to the conversation, and what add reserved for
            // them leaves discovery for the record charge (weigh, and their
            // spare slots in `spare_bytes`). The old list is freed.
            let mut memberships = std::mem::take(&mut assoc.memberships);
            moved = moved
                .saturating_add(memberships.len().saturating_mul(MEMBERSHIP_MAX))
                .saturating_add(memberships.capacity() * size_of::<Membership>());
            memberships.shrink_to_fit();
            conversation.memberships = memberships;
            conversation.evidence_complete = complete(&assoc.sources);
            conversation.reseated = round.reseated.contains(&key.1);
            conversation.gaps.clear();
            if assoc
                .sources
                .iter()
                .any(|source| !round.complete.get(source).copied().unwrap_or(false))
            {
                conversation.gaps.push(GapKind::Unreadable);
            }
            next.push(conversation);
        }
        round.bytes = round.bytes.saturating_sub(moved);
        self.conversations = next;
        let caps = self.caps;
        self.weights = self
            .conversations
            .iter()
            .map(|conversation| weigh(conversation, &caps))
            .collect();
        self.files
            .retain(|place, _| round.assoc.contains_key(&place.conversation));
        let now = holding(
            &self.conversations,
            &self.weights,
            &self.files,
            &self.refused,
        );
        round.held = now.admitted;
        round.pinned = now.pinned;
        queue_locate(round);
    }
}

/// Queue the locate listings of every associated store: one per Claude
/// store, one per Codex day directory, then one find per Codex id.
fn queue_locate(round: &mut Round) {
    let mut claude: BTreeSet<PathBuf> = BTreeSet::new();
    let mut codex: BTreeSet<(PathBuf, String)> = BTreeSet::new();
    for (key, assoc) in &round.assoc {
        for store in &assoc.stores {
            match key.0 {
                Adapter::Claude => {
                    claude.insert(store.clone());
                }
                Adapter::Codex => {
                    codex.insert((store.clone(), key.1.clone()));
                }
            }
        }
    }
    for store in claude {
        round.push(Task::ClaudeStore { store });
    }
    let mut days = BTreeSet::new();
    for (root, id) in &codex {
        if let Some(dirs) = crate::quota::codex_rollout_dirs(root, id) {
            days.extend(dirs);
        } else if round.keep(rollout_bytes(root, id, &Ok(None))) {
            // No id day: proven absent, kept as a rollout outcome.
            round.rollouts.insert((root.clone(), id.clone()), Ok(None));
        }
    }
    for dir in days {
        round.push(Task::CodexDay { dir });
    }
    for (root, id) in codex {
        if !round.rollouts.contains_key(&(root.clone(), id.clone())) {
            round.push(Task::CodexFind { root, id });
        }
    }
}

/// One Codex rollout outcome kept in [`Round::rollouts`].
fn rollout_bytes(root: &Path, id: &str, outcome: &Result<Option<PathBuf>, GapKind>) -> usize {
    ENTRY * size_of::<((PathBuf, String), Result<Option<PathBuf>, GapKind>)>()
        + root.as_os_str().len()
        + id.len()
        + match outcome {
            Ok(Some(path)) => path.as_os_str().len(),
            _ => 0,
        }
}

/// What the conversations hold on the ledger (B3 review r2 B3-1a, r3
/// B3-1a): the admitted ones their charge and their files' state; the
/// refused ones (pinned) the record evict keeps and their refusal entry.
#[derive(Debug, Clone, Copy, Default)]
struct Holding {
    admitted: usize,
    pinned: usize,
}

fn holding(
    conversations: &[Conversation],
    weights: &[Weight],
    files: &BTreeMap<Place, Tracked>,
    refused: &BTreeMap<Key, Refusal>,
) -> Holding {
    let mut sum = Holding::default();
    for (conversation, weight) in conversations.iter().zip(weights) {
        let key = (conversation.adapter, conversation.id.clone());
        let charge = weight.charge.saturating_add(spare_bytes(conversation));
        if !refused.is_empty() && refused.contains_key(&key) {
            sum.pinned = sum
                .pinned
                .saturating_add(charge)
                .saturating_add(refusal_bytes(&key));
        } else {
            sum.admitted = sum.admitted.saturating_add(charge);
        }
    }
    // A refused conversation's files are dropped at eviction.
    sum.admitted = files
        .iter()
        .map(|(place, tracked)| tracked_bytes(place, tracked))
        .fold(sum.admitted, usize::saturating_add);
    sum
}

/// The record charge counts the memberships a conversation uses; their
/// vector's spare slots are retained beside them (B3 review r6 B3-1a).
fn spare_bytes(conversation: &Conversation) -> usize {
    conversation
        .memberships
        .capacity()
        .saturating_sub(conversation.memberships.len())
        * size_of::<Membership>()
}

/// One entry of [`Reader::refused`].
fn refusal_bytes(key: &Key) -> usize {
    ENTRY * size_of::<(Key, Refusal)>() + key.1.len()
}

/// The std B-tree model of the fold's ledger (`tool_stats::fold`'s `TREE`):
/// a map's nodes and edges stay within this many times its entries.
const ENTRY: usize = 3;

/// The `complete` key that says the ledger refused some discovery state.
const LEDGER: &str = "ledger/";

/// One task queue slot.
const SLOT: usize = size_of::<Task>();

/// The fewest slots a growing queue allocates (std grows a queue to at
/// least four slots, then doubles it).
const MIN_SLOTS: usize = 4;

/// The most a vector's allocation may come to once `adding` more elements
/// are pushed one by one: nothing new while they fit, else a doubling (at
/// least [`MIN_SLOTS`] slots, std's least for elements up to 1 KiB) past
/// the length they need. An upper bound for the check; the charge is the
/// capacity actually grown.
fn growth<T>(vec: &Vec<T>, adding: usize) -> usize {
    let need = vec.len().saturating_add(adding);
    if adding == 0 || need <= vec.capacity() {
        0
    } else {
        need.saturating_mul(2)
            .max(MIN_SLOTS)
            .saturating_mul(size_of::<T>())
    }
}

/// What one `Round::add` keeps that `held` (the association so far) does
/// not: its owned bytes, and the most its vectors' allocations may grow.
fn admission(
    held: Option<&Assoc>,
    id: &str,
    source: &str,
    membership: &Membership,
    store: Option<&PathBuf>,
    roots: &Roots,
) -> (usize, usize) {
    let new_path = |path: &PathBuf, held: Option<&Vec<PathBuf>>| {
        if held.is_some_and(|held| held.contains(path)) {
            0
        } else {
            size_of::<PathBuf>() + path.as_os_str().len()
        }
    };
    let entry = if held.is_none() {
        ENTRY * size_of::<(Key, Assoc)>() + id.len()
    } else {
        0
    };
    // The most one membership can own, at a fixed size (lead ruling
    // ae-20261108T170544Z-0d20805a): it has ONE owner, and `associate`
    // moves it, with this charge, to the conversation record.
    let fresh = !held.is_some_and(|held| held.memberships.contains(membership));
    let member = if fresh { MEMBERSHIP_MAX } else { 0 };
    let from = if held.is_some_and(|held| held.sources.contains(source)) {
        0
    } else {
        ENTRY * size_of::<String>() + source.len()
    };
    let located = match store {
        Some(store) if !held.is_some_and(|held| held.stores.contains(store)) => {
            ENTRY * size_of::<PathBuf>() + store.as_os_str().len()
        }
        _ => 0,
    };
    let homes: usize = roots
        .config_homes
        .iter()
        .map(|home| new_path(home, held.map(|held| &held.roots.config_homes)))
        .sum();
    let dirs: usize = roots
        .work_dirs
        .iter()
        .map(|dir| new_path(dir, held.map(|held| &held.roots.work_dirs)))
        .sum();
    // The vectors' spare slots are retained too (B3 review r6
    // B3-1a): reserve the most each may grow by these pushes.
    let empty = Assoc::default();
    let held = held.unwrap_or(&empty);
    let adding =
        |new: &[PathBuf], vec: &Vec<PathBuf>| new.iter().filter(|path| !vec.contains(path)).count();
    let slots = growth(&held.memberships, usize::from(fresh))
        + growth(
            &held.roots.config_homes,
            adding(&roots.config_homes, &held.roots.config_homes),
        )
        + growth(
            &held.roots.work_dirs,
            adding(&roots.work_dirs, &held.roots.work_dirs),
        );
    (entry + member + from + located + homes + dirs, slots)
}

fn roots_bytes(roots: &Roots) -> usize {
    roots
        .config_homes
        .iter()
        .chain(&roots.work_dirs)
        .map(|path| size_of::<PathBuf>() + path.as_os_str().len())
        .sum()
}

/// The slot bytes an association's vectors have allocated.
fn vector_bytes(assoc: &Assoc) -> usize {
    assoc.memberships.capacity() * size_of::<Membership>()
        + (assoc.roots.config_homes.capacity() + assoc.roots.work_dirs.capacity())
            * size_of::<PathBuf>()
}

/// A membership field the reader would retain is longer than [`FIELD_MAX`].
/// The seat's harness binary is not retained (only an [`ident`]-bounded tool
/// name is), so it is not bounded here.
///
/// [`ident`]: crate::tool_stats::ident
fn over_limit(membership: &Membership) -> bool {
    let client = match &membership.client {
        RecordedClient::Label(label) => label.len(),
        RecordedClient::Missing | RecordedClient::Invalid => 0,
    };
    membership.session.as_ref().map_or(0, String::len) > FIELD_MAX
        || membership.slot.len() > FIELD_MAX
        || client > FIELD_MAX
}

fn unsupported_bytes(tool: &str, id: &str, session: Option<&str>) -> usize {
    ENTRY * size_of::<((String, String), BTreeSet<Option<String>>)>()
        + tool.len()
        + id.len()
        + ENTRY * size_of::<Option<String>>()
        + session.map_or(0, str::len)
}

/// The bytes one tracked file keeps beside its conversation: its map entry,
/// its key's owned strings, its path and its roots.
fn tracked_bytes(place: &Place, tracked: &Tracked) -> usize {
    ENTRY * size_of::<(Place, Tracked)>()
        + place_bytes(place)
        + tracked.path.as_os_str().len()
        + roots_bytes(&tracked.roots)
}

/// The names a [`Place`] owns.
fn place_bytes(place: &Place) -> usize {
    place.conversation.1.len()
        + match &place.role {
            StreamRole::Parent => 0,
            StreamRole::Child(name) => name.len(),
        }
}

/// The bytes one queued task owns beyond its queue slot (B3 review r2
/// B3-1a): the paths and names. The slots are charged as the queue's live
/// allocation ([`Round::push`]).
fn task_bytes(task: &Task) -> usize {
    let path = |path: &PathBuf| path.as_os_str().len();
    match task {
        Task::Sessions | Task::Archives => 0,
        Task::Meta { dir, name } => path(dir) + name.len(),
        Task::Journal {
            dir, name, session, ..
        } => path(dir) + name.len() + session.as_ref().map_or(0, String::len),
        Task::Archive { dir, id } => path(dir) + id.len(),
        Task::ClaudeStore { store } | Task::CodexDay { dir: store } => path(store),
        Task::ClaudeProject { store, project } => path(store) + path(project),
        Task::Subagents { store, id, dir } => path(store) + id.len() + path(dir),
        Task::CodexFind { root, id } => path(root) + id.len(),
        Task::Lstats(batch) => batch
            .iter()
            .map(|(place, file)| size_of::<(Place, PathBuf)>() + place_bytes(place) + path(file))
            .sum(),
        Task::Verify { place, .. } => size_of::<ring::digest::Context>() + place_bytes(place),
        Task::Read { place, .. } => place_bytes(place),
    }
}

/// The roots a Claude seat's skill directories classify by.
fn roots_of(entry: &RosterEntry, session_dir: Option<&Path>) -> Roots {
    let mut roots = Roots::default();
    if let RecordedConfigHome::Path(home) | RecordedConfigHome::Implicit(home) = &entry.config_home
    {
        roots.config_homes.push(home.clone());
    }
    match &entry.work_dir {
        RecordedWorkDir::Path(dir) => roots.work_dirs.push(dir.clone()),
        RecordedWorkDir::Missing => roots.work_dirs.extend(session_dir.map(Path::to_owned)),
        RecordedWorkDir::Invalid(_) => {}
    }
    roots
}

/// The adapter a recorded binary is read through; `None` when unsupported.
fn adapter_of(binary: Option<&str>) -> Option<Adapter> {
    match ToolKind::from_binary_name(binary.unwrap_or(""))
        .adapter()
        .usage
        .source
    {
        UsageSource::ClaudeTranscripts => Some(Adapter::Claude),
        UsageSource::CodexRollout => Some(Adapter::Codex),
        UsageSource::Unsupported => None,
    }
}

/// An unsupported seat's tool as the coverage line may keep it (B3 review r1
/// B3-5): the recorded binary's base name only when it passes the plan §3
/// identity rule, else the fixed word `unresolved`. Normalized BEFORE it is
/// retained, so no raw recorded text reaches a [`View`] or its `Debug`.
fn unsupported_tool(binary: Option<&str>) -> String {
    let name = binary
        .and_then(|binary| Path::new(binary).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("");
    match crate::tool_stats::ident(crate::tool_stats::Kind::Tool, name) {
        crate::tool_stats::Ident::Name(name) => name,
        crate::tool_stats::Ident::Unresolved(_) => "unresolved".to_owned(),
    }
}

/// The ids a `reseat` record names: `conversation <id>` or `prior <id>`
/// inside its bracket (plan §5 (c)).
fn reseat_ids(summary: &str) -> Vec<String> {
    let Some(inner) = summary
        .split_once('[')
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(inner, _)| inner)
    else {
        return Vec::new();
    };
    inner
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            part.strip_prefix("conversation ")
                .or_else(|| part.strip_prefix("prior "))
        })
        .filter(|id| crate::archive::is_canonical_uuid(id))
        .map(str::to_owned)
        .collect()
}

impl Round {
    /// Whether `bytes` more discovery state fits the ledger beside what the
    /// refused conversations keep and, while checking, what is held.
    /// Association and location may displace held observations: the unit's
    /// closing [`Reader::enforce`] evicts them, and they are read again
    /// once room frees (B3 review r3 B3-7).
    fn fits(&self, bytes: usize) -> bool {
        let held = if self.phase == Phase::Check {
            self.held
        } else {
            0
        };
        // Association rebuilds every record, so last round's (pinned too)
        // give way to discovery as in a fresh scan; `Reader::displace`
        // removes them after the unit (lead ruling
        // ae-20261108T170544Z-0d20805a).
        let pinned = if self.phase == Phase::Associate {
            0
        } else {
            self.pinned
        };
        self.bytes
            .checked_add(bytes)
            .and_then(|all| all.checked_add(pinned))
            .and_then(|all| all.checked_add(held))
            .is_some_and(|all| all <= self.limit)
    }

    /// Charge `bytes` of discovery state the round is about to retain (plan
    /// §7, B3 review r1 B3-1), BEFORE it is retained. Past the ledger nothing
    /// more is kept and the round's evidence is incomplete, so no client is
    /// proven from it.
    fn keep(&mut self, bytes: usize) -> bool {
        if self.fits(bytes) {
            self.bytes = self.bytes.saturating_add(bytes);
            true
        } else {
            self.complete.insert(LEDGER.to_owned(), false);
            false
        }
    }

    /// Queue `task`, charged until it runs; `false` when the ledger refused
    /// it. The queue's allocation is charged as it is live (B3 review r3
    /// B3-1a): the check reserves a growth by the most the queue may grow
    /// (doubling, at least [`MIN_SLOTS`]), and the charge is then the
    /// capacity it actually has.
    fn push(&mut self, task: Task) -> bool {
        let capacity = self.tasks.capacity();
        let growth = if self.tasks.len() == capacity {
            capacity.max(MIN_SLOTS) * SLOT
        } else {
            0
        };
        let owned = task_bytes(&task);
        if !self.fits(owned.saturating_add(growth)) {
            self.complete.insert(LEDGER.to_owned(), false);
            return false;
        }
        self.tasks.push_back(task);
        let grown = self.tasks.capacity().saturating_sub(capacity) * SLOT;
        self.bytes = self.bytes.saturating_add(owned).saturating_add(grown);
        true
    }

    /// The next task, its charge released; a drained queue releases its
    /// allocation too.
    fn pop(&mut self) -> Option<Task> {
        let task = self.tasks.pop_front()?;
        self.bytes = self.bytes.saturating_sub(task_bytes(&task));
        if self.tasks.is_empty() {
            self.bytes = self.bytes.saturating_sub(self.tasks.capacity() * SLOT);
            self.tasks = VecDeque::new();
        }
        Some(task)
    }

    /// Record whether `source`'s evidence was complete; an incomplete one
    /// never turns complete. A new entry is charged first.
    fn mark(&mut self, source: &str, complete: bool) {
        if let Some(held) = self.complete.get_mut(source) {
            *held &= complete;
        } else if self.keep(ENTRY * size_of::<(String, bool)>() + source.len()) {
            self.complete.insert(source.to_owned(), complete);
        }
    }

    /// Record that `store`'s listing failed or was cut.
    fn broke(&mut self, store: PathBuf) {
        if !self.broken.contains(&store)
            && self.keep(ENTRY * size_of::<PathBuf>() + store.as_os_str().len())
        {
            self.broken.insert(store);
        }
    }

    /// Add one membership of `id` held by `seat`, from evidence `source`.
    /// Only what is new is charged, before it is kept (B3 review r2 B3-1b):
    /// a repeated record adds no discovery state.
    fn add(
        &mut self,
        seat: &RosterEntry,
        id: &str,
        source: &str,
        membership: Membership,
        roots: &Roots,
    ) {
        if over_limit(&membership) {
            self.oversize = self.oversize.saturating_add(1);
            return;
        }
        let Some(adapter) = adapter_of(seat.binary.as_deref()) else {
            let tool = unsupported_tool(seat.binary.as_deref());
            let sessions = self.unsupported.get(&(tool.clone(), id.to_owned()));
            let bytes = match sessions {
                None => unsupported_bytes(&tool, id, membership.session.as_deref()),
                Some(sessions) if sessions.contains(&membership.session) => 0,
                Some(_) => {
                    ENTRY * size_of::<Option<String>>()
                        + membership.session.as_ref().map_or(0, String::len)
                }
            };
            if bytes > 0 && self.keep(bytes) {
                self.unsupported
                    .entry((tool, id.to_owned()))
                    .or_default()
                    .insert(membership.session);
            }
            return;
        };
        let usage = match adapter {
            Adapter::Claude => UsageSource::ClaudeTranscripts,
            Adapter::Codex => UsageSource::CodexRollout,
        };
        // The store as the meta recorded it, resolved without IO (lead ruling
        // ae-20261108T144928Z-d9d1c43b); none leaves the conversation listed
        // and unlocated.
        let store = super::source_lexical(seat, usage, self.home.as_deref()).ok();
        let key = (adapter, id.to_owned());
        let (bytes, slots) = admission(
            self.assoc.get(&key),
            id,
            source,
            &membership,
            store.as_ref(),
            roots,
        );
        if (bytes > 0 || slots > 0) && !self.fits(bytes.saturating_add(slots)) {
            self.complete.insert(LEDGER.to_owned(), false);
            return;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        let assoc = self.assoc.entry(key).or_default();
        let before = vector_bytes(assoc);
        if !assoc.memberships.contains(&membership) {
            assoc.memberships.push(membership);
        }
        assoc.sources.insert(source.to_owned());
        if let Some(store) = store {
            assoc.stores.insert(store);
        }
        for home in &roots.config_homes {
            if !assoc.roots.config_homes.contains(home) {
                assoc.roots.config_homes.push(home.clone());
            }
        }
        for dir in &roots.work_dirs {
            if !assoc.roots.work_dirs.contains(dir) {
                assoc.roots.work_dirs.push(dir.clone());
            }
        }
        assoc.roots.config_homes.sort();
        assoc.roots.work_dirs.sort();
        let grown = vector_bytes(assoc).saturating_sub(before);
        self.bytes = self.bytes.saturating_add(grown);
    }

    /// One journal record: `retire` adds a retired membership, `reseat`
    /// names the ids it moved.
    fn record(&mut self, event: &crate::events::Event, source: &str, session: Option<&str>) {
        match event.action.as_str() {
            "reseat" => {
                for id in reseat_ids(event.summary.as_deref().unwrap_or("")) {
                    if !self.reseated.contains(&id)
                        && self.keep(ENTRY * size_of::<String>() + id.len())
                    {
                        self.reseated.insert(id);
                    }
                }
            }
            "retire" => {
                let Some(id) = super::valid_id(event.reference.as_deref()) else {
                    return;
                };
                let Some((tool, profile, home_row, base)) =
                    event.summary.as_deref().and_then(super::retired_fields)
                else {
                    return;
                };
                let slot = event.target_slot.value().unwrap_or("?").to_owned();
                let seat = RosterEntry {
                    slot: slot.clone(),
                    name: String::new(),
                    profile: Some(profile),
                    client: RecordedClient::Missing,
                    harness_session: Some(id.clone()),
                    config_home: if home_row.is_empty() {
                        RecordedConfigHome::Missing
                    } else {
                        RecordedConfigHome::parse(&home_row)
                    },
                    config_home_base: if base.is_empty() {
                        crate::meta::RecordedConfigHomeBase::Missing
                    } else {
                        crate::meta::RecordedConfigHomeBase::parse(&base)
                    },
                    binary: Some(tool),
                    work_dir: RecordedWorkDir::Missing,
                };
                let roots = roots_of(&seat, None);
                let membership = Membership {
                    session: session.map(str::to_owned),
                    slot,
                    role: Role::Retired,
                    client: RecordedClient::Missing,
                };
                self.add(&seat, &id, source, membership, &roots);
            }
            _ => {}
        }
    }
}

// --- Locating (plan §6) ----------------------------------------------------

impl Reader {
    fn claude_store(&mut self, store: PathBuf) {
        let Some(round) = self.round.as_mut() else {
            return;
        };
        let mut budget = Budget::local(u64::MAX);
        let projects = match super::derived_directory(&store, "projects", &mut budget) {
            Ok(Some(projects)) => projects,
            Ok(None) => return,
            Err(_) => {
                round.broke(store);
                return;
            }
        };
        match super::names_only(&projects, &mut budget) {
            Ok(Bounded::Ready(names)) => {
                for (name, kind) in names {
                    if kind == Entry::Dir {
                        round.push(Task::ClaudeProject {
                            store: store.clone(),
                            project: projects.join(name),
                        });
                    }
                }
            }
            Ok(Bounded::Truncated) | Err(_) => {
                round.broke(store);
            }
        }
    }

    fn claude_project(&mut self, store: PathBuf, project: &Path) {
        let Some(round) = self.round.as_mut() else {
            return;
        };
        let names = match super::names_only(project, &mut Budget::local(u64::MAX)) {
            Ok(Bounded::Ready(names)) => names,
            Ok(Bounded::Truncated) | Err(_) => {
                round.broke(store);
                return;
            }
        };
        let wanted: BTreeSet<&str> = round
            .assoc
            .iter()
            .filter(|((adapter, _), assoc)| {
                *adapter == Adapter::Claude && assoc.stores.contains(&store)
            })
            .map(|((_, id), _)| id.as_str())
            .collect();
        let mut found = Vec::new();
        let mut children = Vec::new();
        for (name, kind) in names {
            match kind {
                Entry::File => {
                    if let Some(id) = name.strip_suffix(".jsonl")
                        && wanted.contains(id)
                    {
                        found.push((id.to_owned(), project.join(&name)));
                    }
                }
                Entry::Dir if wanted.contains(name.as_str()) => {
                    children.push((name.clone(), project.join(&name)));
                }
                Entry::Dir | Entry::Other => {}
            }
        }
        for (id, path) in found {
            let bytes = ENTRY * size_of::<((PathBuf, String), Vec<PathBuf>)>()
                + store.as_os_str().len()
                + id.len()
                + size_of::<PathBuf>()
                + path.as_os_str().len();
            if round.keep(bytes) {
                round
                    .parents
                    .entry((store.clone(), id))
                    .or_default()
                    .push(path);
            }
        }
        for (id, dir) in children {
            round.push(Task::Subagents {
                store: store.clone(),
                id,
                dir,
            });
        }
    }

    fn subagents(&mut self, store: PathBuf, id: String, dir: &Path) {
        let Some(round) = self.round.as_mut() else {
            return;
        };
        let mut budget = Budget::local(u64::MAX);
        let subagents = match super::derived_directory(dir, "subagents", &mut budget) {
            Ok(Some(subagents)) => subagents,
            Ok(None) => return,
            Err(_) => {
                round.broke(store);
                return;
            }
        };
        match super::names_only(&subagents, &mut budget) {
            Ok(Bounded::Ready(names)) => {
                let mut kept = BTreeMap::new();
                for (name, kind) in names {
                    if kind == Entry::File
                        && let Some(stem) = name.strip_suffix(".jsonl")
                    {
                        let path = subagents.join(&name);
                        let bytes = ENTRY * size_of::<(String, PathBuf)>()
                            + stem.len()
                            + path.as_os_str().len();
                        if round.keep(bytes) {
                            kept.insert(stem.to_owned(), path);
                        }
                    }
                }
                let bytes = ENTRY * size_of::<((PathBuf, String), BTreeMap<String, PathBuf>)>()
                    + store.as_os_str().len()
                    + id.len();
                if round.keep(bytes) {
                    round.children.entry((store, id)).or_default().extend(kept);
                }
            }
            Ok(Bounded::Truncated) | Err(_) => {
                round.broke(store);
            }
        }
    }

    fn codex_day(&mut self, dir: &Path) {
        let Some(round) = self.round.as_mut() else {
            return;
        };
        // The cache keeps a complete listing (or a proven absence); a failed
        // or cut one is not cached, and the find then reports its ids
        // unreadable without listing it again.
        let kept = match round.days.names(dir, &mut Budget::local(u64::MAX)) {
            Ok(Bounded::Ready(names)) => names.map_or(0, |names| {
                names
                    .iter()
                    .map(|name| size_of::<std::ffi::OsString>() + name.len())
                    .sum::<usize>()
            }),
            Ok(Bounded::Truncated) | Err(_) => return,
        };
        // The listing is charged before it counts as kept (B3 review r2
        // B3-1a): one the ledger refuses is released at once, and its ids
        // then read as unreadable.
        let bytes = ENTRY * size_of::<(PathBuf, Option<Vec<std::ffi::OsString>>)>()
            + dir.as_os_str().len()
            + kept
            + ENTRY * size_of::<PathBuf>()
            + dir.as_os_str().len();
        if round.keep(bytes) {
            round.listed_days.insert(dir.to_owned());
        } else {
            round.days.forget(dir);
        }
    }

    fn codex_find(&mut self, root: PathBuf, id: String) {
        let Some(round) = self.round.as_mut() else {
            return;
        };
        // The find only consults day listings its own units cached: a day
        // whose listing failed or was cut is never listed again inside this
        // lstat unit (one listing per unit at most, B3 review r1 B3-2).
        let cached = crate::quota::codex_rollout_dirs(&root, &id)
            .is_some_and(|dirs| dirs.iter().all(|dir| round.listed_days.contains(dir)));
        let found = if cached {
            crate::quota::find_codex_rollout_listed(
                &root,
                &id,
                &mut Budget::local(u64::MAX),
                &mut round.days,
            )
        } else {
            Ok(Bounded::Truncated)
        };
        let outcome = match found {
            Ok(Bounded::Ready(Some(file))) => Ok(Some(file.path().to_owned())),
            Ok(Bounded::Ready(None)) => Ok(None),
            Err(error) if error.to_string().contains("not unique") => Err(GapKind::NotUnique),
            Ok(Bounded::Truncated) | Err(_) => Err(GapKind::Unreadable),
        };
        if round.keep(rollout_bytes(&root, &id, &outcome)) {
            round.rollouts.insert((root, id), outcome);
        }
    }

    /// Locating is done: give every located file its place and stream,
    /// record unlocated and non-unique conversations, queue the lstats.
    fn place(&mut self) {
        let Some(round) = self.round.as_mut() else {
            return;
        };
        let (located, with_parent) = locate(round, &mut self.conversations);
        self.files
            .retain(|place, tracked| located.get(place).is_some_and(|path| *path == tracked.path));
        let files = &self.files;
        self.recounting.retain(|place| files.contains_key(place));
        // The lstats are discovery: they may displace held observations,
        // which the closing enforce evicts and a later readmission reads
        // again (B3 review r3 B3-7). A conversation whose lstats the ledger
        // cannot queue at all is refused, visibly and recoverably.
        let mut short: BTreeSet<Key> = BTreeSet::new();
        let batch: Vec<(Place, PathBuf)> = located.into_iter().collect();
        for chunk in batch.chunks(usize::try_from(LSTATS).unwrap_or(256)) {
            if !round.push(Task::Lstats(chunk.to_vec())) {
                short.extend(chunk.iter().map(|(place, _)| place.conversation.clone()));
            }
        }
        // Discovery state the ledger refused may hide a membership of any
        // conversation: no client is proven this round.
        if round.complete.get(LEDGER) == Some(&false) {
            self.hidden = round
                .complete
                .values()
                .filter(|complete| !**complete)
                .count()
                + usize::from(!round.archives_listed);
            for conversation in &mut self.conversations {
                conversation.evidence_complete = false;
            }
        }
        // Read only when located and every located file was read to its end;
        // the lstats that follow clear it for any file they queue.
        for index in 0..self.conversations.len() {
            let Some(key) = self
                .conversations
                .get(index)
                .map(|conversation| (conversation.adapter, conversation.id.clone()))
            else {
                continue;
            };
            let read = with_parent.contains(&key) && self.settled(&key);
            if let Some(conversation) = self.conversations.get_mut(index) {
                conversation.read = read;
            }
        }
        // Streams and file state changed: the ledger decides again before
        // any read of this round.
        self.reweigh_all();
        for key in short {
            if let Some(index) = self.index(&key) {
                self.evict(index);
            }
        }
        self.enforce();
    }

    /// Whether every located file of `key` is tracked and was read to its end.
    fn settled(&self, key: &Key) -> bool {
        let Ok(at) = self.conversations.binary_search_by(|seen| {
            (seen.adapter, seen.id.as_str()).cmp(&(key.0, key.1.as_str()))
        }) else {
            return false;
        };
        self.conversations.get(at).is_some_and(|conversation| {
            conversation.streams.iter().all(|stream| {
                let place = Place {
                    conversation: key.clone(),
                    role: stream.role.clone(),
                    copy: stream.copy,
                };
                self.files.get(&place).is_some_and(|tracked| tracked.done)
            })
        })
    }
}

/// What locating found for one conversation.
struct Found {
    files: Vec<(Place, PathBuf)>,
    parent: bool,
    gaps: Vec<GapKind>,
}

/// The files the round's listings located for conversation `key`, whether
/// one is its parent, and the gaps its stores show. Pure over the round's
/// discovery state, so a readmission locates the same files again (B3
/// review r3 B3-7).
fn found(round: &Round, key: &Key, assoc: &Assoc) -> Found {
    let mut found = Found {
        files: Vec::new(),
        parent: false,
        gaps: Vec::new(),
    };
    for (copy, store) in (0_u32..).zip(&assoc.stores) {
        if round.broken.contains(store) {
            found.gaps.push(GapKind::Unreadable);
            continue;
        }
        let at = |role: StreamRole| Place {
            conversation: key.clone(),
            role,
            copy,
        };
        match key.0 {
            Adapter::Claude => {
                let pair = (store.clone(), key.1.clone());
                match round.parents.get(&pair).map(Vec::as_slice) {
                    Some([path]) => {
                        found.parent = true;
                        found.files.push((at(StreamRole::Parent), path.clone()));
                    }
                    Some([_, _, ..]) => found.gaps.push(GapKind::NotUnique),
                    _ => {}
                }
                for (name, path) in round.children.get(&pair).into_iter().flatten() {
                    found
                        .files
                        .push((at(StreamRole::Child(name.clone())), path.clone()));
                }
            }
            Adapter::Codex => match round.rollouts.get(&(store.clone(), key.1.clone())) {
                Some(Ok(Some(path))) => {
                    found.parent = true;
                    found.files.push((at(StreamRole::Parent), path.clone()));
                }
                Some(Err(gap)) => found.gaps.push(gap.clone()),
                _ => {}
            },
        }
    }
    found
}

/// Each associated conversation's located files by place, and the
/// conversations with a parent file; marks the unlocated, unreadable and
/// non-unique ones and lines their streams up with the located files.
fn locate(
    round: &Round,
    conversations: &mut [Conversation],
) -> (BTreeMap<Place, PathBuf>, BTreeSet<Key>) {
    let mut located: BTreeMap<Place, PathBuf> = BTreeMap::new();
    let mut with_parent: BTreeSet<Key> = BTreeSet::new();
    for conversation in conversations {
        let key = (conversation.adapter, conversation.id.clone());
        let Some(assoc) = round.assoc.get(&key) else {
            continue;
        };
        let found = found(round, &key, assoc);
        conversation.gaps.extend(found.gaps);
        if !found.parent && !conversation.gaps.contains(&GapKind::NotUnique) {
            conversation.gaps.push(GapKind::Unlocated);
        }
        conversation.gaps.sort();
        conversation.gaps.dedup();
        if found.parent {
            with_parent.insert(key.clone());
        }
        conversation.streams.retain(|stream| {
            found
                .files
                .iter()
                .any(|(place, _)| place.role == stream.role && place.copy == stream.copy)
        });
        for (place, path) in found.files {
            let present = conversation
                .streams
                .iter()
                .any(|stream| stream.role == place.role && stream.copy == place.copy);
            if !present {
                conversation.streams.push(Stream {
                    role: place.role.clone(),
                    copy: place.copy,
                    events: Vec::new(),
                    malformed: 0,
                    over_cap: 0,
                });
            }
            located.insert(place, path);
        }
    }
    (located, with_parent)
}

// --- Ledger (plan §7, B3 review r1 B3-1) -------------------------------------

impl Reader {
    /// The ledger the admitted conversations share: the cap less the fixed
    /// view reserve, the discovery state kept beside them (the round's, or
    /// between rounds the unsupported coverage) and what the refused ones
    /// keep.
    fn budget(&self) -> usize {
        self.caps
            .ledger
            .saturating_sub(VIEW_RESERVE)
            .saturating_sub(self.discovery())
            .saturating_sub(self.pinned())
    }

    /// The discovery state kept beside the conversations: the round's, or
    /// between rounds the unsupported coverage.
    fn discovery(&self) -> usize {
        self.round.as_ref().map_or_else(
            || {
                self.unsupported
                    .iter()
                    .map(|((tool, id), sessions)| {
                        sessions
                            .iter()
                            .map(|session| unsupported_bytes(tool, id, session.as_deref()))
                            .sum::<usize>()
                    })
                    .sum()
            },
            |round| round.bytes,
        )
    }

    /// What the refused conversations keep: the record evict leaves and
    /// their refusal entries (B3 review r3 B3-1a).
    fn pinned(&self) -> usize {
        self.refused
            .keys()
            .map(|key| {
                self.index(key)
                    .and_then(|index| {
                        let weight = self.weights.get(index)?;
                        let conversation = self.conversations.get(index)?;
                        Some(weight.charge.saturating_add(spare_bytes(conversation)))
                    })
                    .unwrap_or(0)
                    .saturating_add(refusal_bytes(key))
            })
            .fold(0_usize, usize::saturating_add)
    }

    fn index(&self, key: &Key) -> Option<usize> {
        self.conversations
            .binary_search_by(|seen| (seen.adapter, seen.id.as_str()).cmp(&(key.0, key.1.as_str())))
            .ok()
    }

    /// Recompute every weight (the conversations or their streams changed).
    fn reweigh_all(&mut self) {
        let caps = self.caps;
        self.weights = self
            .conversations
            .iter()
            .map(|conversation| weigh(conversation, &caps))
            .collect();
    }

    /// Recompute one conversation's weight after its evidence changed.
    fn reweigh(&mut self, key: &Key) {
        let caps = self.caps;
        if let Some(index) = self.index(key)
            && let Some(conversation) = self.conversations.get(index)
            && let Some(weight) = self.weights.get_mut(index)
        {
            *weight = weigh(conversation, &caps);
        }
    }

    /// What the conversations hold now.
    fn holding(&self) -> Holding {
        holding(
            &self.conversations,
            &self.weights,
            &self.files,
            &self.refused,
        )
    }

    /// Everything the conversations hold on the ledger, admitted or pinned.
    fn held(&self) -> usize {
        let now = self.holding();
        now.admitted.saturating_add(now.pinned)
    }

    /// Per conversation in `(adapter, id)` order: the file-state bytes kept
    /// beside it.
    fn kept(&self) -> Vec<usize> {
        let mut kept = vec![0_usize; self.conversations.len()];
        for (place, tracked) in &self.files {
            if let Some(slot) = self
                .index(&place.conversation)
                .and_then(|index| kept.get_mut(index))
            {
                *slot = slot.saturating_add(tracked_bytes(place, tracked));
            }
        }
        kept
    }

    /// The refusal of conversation `index`, if any.
    fn refusal(&self, index: usize) -> Option<Refusal> {
        if self.refused.is_empty() {
            return None;
        }
        let conversation = self.conversations.get(index)?;
        self.refused
            .get(&(conversation.adapter, conversation.id.clone()))
            .copied()
    }

    /// Per conversation in `(adapter, id)` order: its weight, a refused one
    /// weighing as refused, and the file-state bytes kept beside it.
    fn weighed(&self) -> Vec<(Weight, usize)> {
        self.weights
            .iter()
            .zip(self.kept())
            .enumerate()
            .map(|(index, (weight, kept))| {
                let weight = match self.refusal(index) {
                    Some(Refusal::Alone) => Weight {
                        charge: 0,
                        alone: true,
                    },
                    Some(Refusal::Prefix { .. }) => Weight {
                        charge: usize::MAX,
                        alone: false,
                    },
                    None => Weight {
                        charge: weight
                            .charge
                            .saturating_add(self.conversations.get(index).map_or(0, spare_bytes)),
                        alone: weight.alone,
                    },
                };
                (weight, kept)
            })
            .collect()
    }

    /// The ledger left before conversation `index`, the readmission measure
    /// (B3 review r3 B3-7): the cap less the view reserve, the discovery
    /// state and what every EARLIER conversation holds (its charge and files
    /// when admitted, its record and refusal entry when refused). It leaves
    /// out the conversation itself and the ones after it, whose refusal
    /// status changes as it is refused, so it grows only when discovery is
    /// released or an earlier conversation shrinks.
    fn room_before(&self, index: usize) -> usize {
        let used = self
            .weights
            .iter()
            .zip(self.kept())
            .enumerate()
            .take(index)
            .map(|(at, (weight, kept))| self.holds(at, *weight, kept))
            .fold(0_usize, usize::saturating_add);
        self.caps
            .ledger
            .saturating_sub(VIEW_RESERVE)
            .saturating_sub(self.discovery())
            .saturating_sub(used)
    }

    /// What conversation `index` holds on the ledger: its record and refusal
    /// entry when refused, else its charge and files; its membership
    /// vector's spare slots either way.
    fn holds(&self, index: usize, weight: Weight, kept: usize) -> usize {
        let Some(conversation) = self.conversations.get(index) else {
            return 0;
        };
        let own = if self.refusal(index).is_some() {
            refusal_bytes(&(conversation.adapter, conversation.id.clone()))
        } else {
            kept
        };
        weight
            .charge
            .saturating_add(spare_bytes(conversation))
            .saturating_add(own)
    }

    /// Association (lead ruling ae-20261108T170544Z-0d20805a): every
    /// conversation is last round's and is rebuilt at `associate` from this
    /// round's association alone, so discovery takes its room as in a fresh
    /// scan (`Round::fits` leaves the records out in this phase). After each
    /// unit the longest `(adapter, id)` prefix whose holdings fit beside
    /// discovery stays as a cache; the rest are removed WHOLE (record,
    /// memberships, files, refusal), so no conversation is ever kept without
    /// its memberships and none reaches a View that way.
    fn displace(&mut self) {
        let room = self
            .caps
            .ledger
            .saturating_sub(VIEW_RESERVE)
            .saturating_sub(self.discovery());
        let kept = self.kept();
        let mut used = 0_usize;
        let mut cut = self.conversations.len();
        for (index, (weight, kept)) in self.weights.iter().zip(kept).enumerate() {
            let holds = self.holds(index, *weight, kept);
            if used.saturating_add(holds) > room {
                cut = index;
                break;
            }
            used = used.saturating_add(holds);
        }
        if cut >= self.conversations.len() {
            return;
        }
        let gone: BTreeSet<Key> = self
            .conversations
            .drain(cut..)
            .map(|conversation| (conversation.adapter, conversation.id))
            .collect();
        self.weights.truncate(cut);
        self.files
            .retain(|place, _| !gone.contains(&place.conversation));
        self.recounting
            .retain(|place| !gone.contains(&place.conversation));
        self.refused.retain(|key, _| !gone.contains(key));
    }

    /// Evict every conversation the shared ledger refuses now. An eviction
    /// pins the record it keeps, which narrows the budget, so this repeats
    /// until nothing more is refused.
    fn enforce(&mut self) {
        if self
            .round
            .as_ref()
            .is_some_and(|round| round.phase == Phase::Associate)
        {
            self.displace();
            return;
        }
        loop {
            let admitted = prefix(self.weighed(), self.budget());
            let mut evicted = false;
            for (index, admitted) in admitted.into_iter().enumerate() {
                if !admitted && self.refusal(index).is_none() {
                    self.evict(index);
                    evicted = true;
                }
            }
            if !evicted {
                return;
            }
        }
    }

    /// The queue drained: read again the first conversation the prefix rule
    /// refused, when the room before it grew since (B3 review r2 B3-7, r3
    /// B3-7). One refused during its own readmission is tried again only
    /// after a later shrink, so the round ends. Its files are located again
    /// from the round's listings. `false` when none may be read again, so
    /// the round is over.
    fn readmit(&mut self) -> bool {
        let first = (0..self.conversations.len()).find_map(|index| match self.refusal(index) {
            Some(Refusal::Prefix { room, settled }) => Some((index, room, settled)),
            Some(Refusal::Alone) | None => None,
        });
        let Some((index, room, settled)) = first else {
            return false;
        };
        let shrinks = self.shrinks;
        if self.room_before(index) <= room || settled.is_some_and(|at| at >= shrinks) {
            return false;
        }
        let Some(conversation) = self.conversations.get_mut(index) else {
            return false;
        };
        let key = (conversation.adapter, conversation.id.clone());
        conversation.gaps.retain(|gap| *gap != GapKind::Ledger);
        self.refused.remove(&key);
        self.reweigh(&key);
        self.retrying = Some(index);
        let files = self
            .round
            .as_ref()
            .and_then(|round| {
                round
                    .assoc
                    .get(&key)
                    .map(|assoc| found(round, &key, assoc).files)
            })
            .unwrap_or_default();
        for chunk in files.chunks(usize::try_from(LSTATS).unwrap_or(256)) {
            if !self.push(Task::Lstats(chunk.to_vec())) {
                self.evict(index);
                break;
            }
        }
        true
    }

    /// Queue `task` on the round, charged against the ledger as it stands
    /// now; `false` when the ledger refused it (or no round runs).
    fn push(&mut self, task: Task) -> bool {
        let now = self.holding();
        self.round.as_mut().is_some_and(|round| {
            round.held = now.admitted;
            round.pinned = now.pinned;
            round.push(task)
        })
    }

    /// The bytes conversation `index` may still gather before the ledger
    /// refuses it; `None` when it is refused already.
    fn room(&self, index: usize) -> Option<usize> {
        let weighed: Vec<(Weight, usize)> = self.weighed().into_iter().take(index + 1).collect();
        let admitted = prefix(weighed.iter().copied(), self.budget());
        if !admitted.last().copied().unwrap_or(false) {
            return None;
        }
        let used = weighed
            .iter()
            .zip(&admitted)
            .filter(|(_, admitted)| **admitted)
            .map(|((weight, kept), _)| weight.charge.saturating_add(*kept))
            .fold(0_usize, usize::saturating_add);
        Some(self.budget().saturating_sub(used))
    }

    /// Refuse conversation `index`: drop its observations and its files'
    /// state and keep a path-free `over cap` line. Only [`Self::readmit`]
    /// reads it again this round.
    fn evict(&mut self, index: usize) {
        if self.refusal(index).is_some() {
            return;
        }
        let caps = self.caps;
        let refusal = if self.weights.get(index).is_some_and(|weight| weight.alone) {
            // What it held is free for the conversations after it.
            self.shrinks = self.shrinks.saturating_add(1);
            Refusal::Alone
        } else {
            let cascade =
                (0..index).any(|at| matches!(self.refusal(at), Some(Refusal::Prefix { .. })));
            Refusal::Prefix {
                room: if cascade { 0 } else { self.room_before(index) },
                settled: (self.retrying == Some(index)).then_some(self.shrinks),
            }
        };
        let Some(conversation) = self.conversations.get_mut(index) else {
            return;
        };
        let key = (conversation.adapter, conversation.id.clone());
        for stream in &mut conversation.streams {
            stream.events = Vec::new();
            stream.malformed = 0;
            stream.over_cap = 0;
        }
        if !conversation.gaps.contains(&GapKind::Ledger) {
            conversation.gaps.push(GapKind::Ledger);
            conversation.gaps.sort();
        }
        conversation.read = false;
        if let Some(weight) = self.weights.get_mut(index) {
            *weight = weigh(conversation, &caps);
        }
        self.files.retain(|place, _| place.conversation != key);
        self.recounting.retain(|place| place.conversation != key);
        self.refused.insert(key, refusal);
    }
}

// --- Reading (plan §6) -----------------------------------------------------

impl Reader {
    fn lstats(&mut self, batch: Vec<(Place, PathBuf)>, guard: &Guard) {
        let mut batch = batch.into_iter();
        while let Some((place, path)) = batch.next() {
            if self.refused.contains_key(&place.conversation) {
                continue;
            }
            // One lstat per located file: the rest waits for its own unit
            // once this one's receipt reaches the bound.
            if stats(guard) >= u64::from(LSTATS) {
                let rest: Vec<(Place, PathBuf)> =
                    std::iter::once((place, path)).chain(batch).collect();
                self.requeue(rest);
                break;
            }
            let found = super::locate_transcript(&path, &mut Budget::local(u64::MAX));
            let Ok(Some(file)) = found else {
                // Gone or refused since it was listed: unreadable this round.
                self.reset(&place);
                self.files.remove(&place);
                self.recounting.remove(&place);
                if let Some(conversation) = self.conversation(&place) {
                    conversation.read = false;
                    if !conversation.gaps.contains(&GapKind::Unreadable) {
                        conversation.gaps.push(GapKind::Unreadable);
                    }
                }
                continue;
            };
            let identity = Identity::of(&file.metadata);
            let roots = self
                .round
                .as_ref()
                .and_then(|round| round.assoc.get(&place.conversation))
                .map_or_else(Roots::default, |assoc| assoc.roots.clone());
            let fresh = match self.files.get(&place) {
                None => true,
                Some(tracked) => {
                    tracked.roots != roots
                        || tracked.identity.dev != identity.dev
                        || tracked.identity.ino != identity.ino
                        || identity.len < tracked.lines.cursor()
                }
            };
            if fresh {
                // Reset first: a tracked file with a cursor is a recount.
                self.reset(&place);
                self.files.insert(place.clone(), Tracked::new(path, roots));
                self.queue(Task::Read {
                    place,
                    anchor: Some(identity),
                });
                continue;
            }
            let Some(tracked) = self.files.get(&place) else {
                continue;
            };
            if tracked.done && tracked.identity == identity {
                continue;
            }
            if tracked.lines.cursor() == 0 {
                self.queue(Task::Read {
                    place,
                    anchor: Some(identity),
                });
            } else {
                let hash = Box::new(ring::digest::Context::new(&ring::digest::SHA256));
                self.queue(Task::Verify {
                    place,
                    at: 0,
                    hash,
                    anchor: identity,
                });
            }
        }
        // The new file state is held on the ledger beside its conversation.
        self.enforce();
    }

    /// Queue the lstats of a batch's `rest` for a later unit; a conversation
    /// whose files the ledger cannot keep queued is refused, visibly.
    fn requeue(&mut self, rest: Vec<(Place, PathBuf)>) {
        let owners: BTreeSet<Key> = rest
            .iter()
            .map(|(place, _)| place.conversation.clone())
            .collect();
        if !self.push(Task::Lstats(rest)) {
            for key in owners {
                if let Some(index) = self.index(&key) {
                    self.evict(index);
                }
            }
        }
    }

    fn queue(&mut self, task: Task) {
        let owner = match &task {
            Task::Verify { place, .. } | Task::Read { place, .. } => Some(place.clone()),
            _ => None,
        };
        if let Some(place) = &owner {
            if let Some(tracked) = self.files.get_mut(place) {
                tracked.done = false;
            }
            if let Some(conversation) = self.conversation(place) {
                conversation.read = false;
            }
        }
        // A task the ledger cannot keep refuses its conversation, visibly.
        if !self.push(task)
            && let Some(index) = owner.and_then(|place| self.index(&place.conversation))
        {
            self.evict(index);
        }
    }

    fn conversation(&mut self, place: &Place) -> Option<&mut Conversation> {
        let (adapter, id) = &place.conversation;
        self.conversations
            .binary_search_by(|seen| (seen.adapter, seen.id.as_str()).cmp(&(*adapter, id.as_str())))
            .ok()
            .and_then(|at| self.conversations.get_mut(at))
    }

    fn stream(&mut self, place: &Place) -> Option<&mut Stream> {
        let (role, copy) = (place.role.clone(), place.copy);
        self.conversation(place)?
            .streams
            .iter_mut()
            .find(|stream| stream.role == role && stream.copy == copy)
    }

    /// Drop a file's observations and read it again from 0.
    fn reset(&mut self, place: &Place) {
        if let Some(tracked) = self.files.get_mut(place) {
            let had = tracked.lines.cursor() > 0;
            tracked.lines = Lines::new(LINE_CAP);
            tracked.hash = ring::digest::Context::new(&ring::digest::SHA256);
            tracked.identity = Identity::default();
            tracked.done = false;
            if had {
                self.recounting.insert(place.clone());
                self.shrinks = self.shrinks.saturating_add(1);
            }
        }
        if let Some(stream) = self.stream(place) {
            stream.events = Vec::new();
            stream.malformed = 0;
            stream.over_cap = 0;
        }
        self.reweigh(&place.conversation);
    }

    /// One chunk re-hashing the consumed prefix `[at, cursor)`. Every chunk
    /// must see the status `anchor` the pass started from (B3 review r1
    /// B3-3); a file that changed under the pass ends it as changed, and the
    /// next round verifies again from 0.
    fn verify(
        &mut self,
        place: Place,
        at: u64,
        hash: Box<ring::digest::Context>,
        anchor: Identity,
    ) {
        let mut hash = *hash;
        let Some(tracked) = self.files.get(&place) else {
            return;
        };
        let cursor = tracked.lines.cursor();
        let window = super::journal_window(&tracked.path, at, STEP.min(cursor.saturating_sub(at)));
        let Ok(window) = window else {
            self.finish(&place, false);
            return;
        };
        if Identity::window(&window) != anchor {
            self.finish(&place, false);
            return;
        }
        hash.update(&window.bytes);
        let at = at.saturating_add(u64::try_from(window.bytes.len()).unwrap_or(u64::MAX));
        if window.bytes.is_empty() && at < cursor {
            self.reset(&place);
            self.queue(Task::Read {
                place,
                anchor: Some(anchor),
            });
            return;
        }
        if at < cursor {
            self.queue(Task::Verify {
                place,
                at,
                hash: Box::new(hash),
                anchor,
            });
            return;
        }
        let same = self.files.get(&place).is_some_and(|tracked| {
            tracked.hash.clone().finish().as_ref() == hash.finish().as_ref()
        });
        if !same {
            self.reset(&place);
        }
        // The read resumes under the status the prefix was verified at.
        self.queue(Task::Read {
            place,
            anchor: Some(anchor),
        });
    }

    /// One chunk of new bytes from the cursor. With an `anchor`, the chunk
    /// must see that status (the lstat's, the verify's or the previous
    /// chunk's); otherwise the file changed between units: the chunk is
    /// dropped, the file ends this round as changed with its last
    /// consistent status kept, so the next round verifies the consumed
    /// prefix (B3 review r1 B3-3).
    fn read(&mut self, place: Place, anchor: Option<Identity>) {
        let Some(index) = self.index(&place.conversation) else {
            return;
        };
        // The ledger first (plan §7, B3 review r1 B3-1): a conversation the
        // prefix already refuses is evicted without reading.
        let Some(room) = self.room(index) else {
            self.evict(index);
            return;
        };
        let Some(tracked) = self.files.get_mut(&place) else {
            return;
        };
        let window = super::journal_window(&tracked.path, tracked.lines.cursor(), STEP);
        let Ok(window) = window else {
            self.finish(&place, false);
            return;
        };
        let identity = Identity::window(&window);
        if anchor.is_some_and(|anchor| anchor != identity) {
            self.finish(&place, false);
            return;
        }
        let adapter = place.conversation.0;
        let roots = tracked.roots.clone();
        let mut lines = tracked.lines.clone();
        // The stream's held events (sorted, distinct) leave it while the
        // chunk is gathered, so a new event's floor counts only once.
        let held = self
            .stream(&place)
            .map(|stream| std::mem::take(&mut stream.events))
            .unwrap_or_default();
        // Each new distinct event once: one already held or already gathered
        // from this chunk adds nothing to the floor (B3 review r2 B3-1b).
        let mut gathered: BTreeSet<Event> = BTreeSet::new();
        let mut parsed_events = Vec::new();
        let mut malformed = 0_u64;
        let mut over_cap = 0_u64;
        let mut floor = 0_usize;
        let mut over = false;
        let consumed = {
            let claude = claude_calls::Roots {
                config_homes: &roots.config_homes,
                work_dirs: &roots.work_dirs,
            };
            lines.step(&window.bytes, &mut |framed| match framed {
                Framed::OverCap => over_cap += 1,
                Framed::Line(line) => {
                    let parsed = match adapter {
                        Adapter::Claude => claude_calls::line(line, &claude, &mut parsed_events),
                        Adapter::Codex => codex_calls::line(line, &mut parsed_events),
                    };
                    if parsed == Parsed::Malformed {
                        malformed += 1;
                    }
                    for event in parsed_events.drain(..) {
                        if held.binary_search(&event).is_err() && !gathered.contains(&event) {
                            floor = floor.saturating_add(event_floor(&event));
                            gathered.insert(event);
                        }
                    }
                    // Past its room by the events' own floor, the conversation
                    // is refused whatever else it holds: stop gathering, so
                    // the chunk never holds more than its room plus one line.
                    over |= floor > room;
                    if over {
                        gathered.clear();
                    }
                }
            })
        };
        if over {
            self.evict(index);
            return;
        }
        let Some(tracked) = self.files.get_mut(&place) else {
            return;
        };
        let used = usize::try_from(consumed).unwrap_or(window.bytes.len());
        tracked
            .hash
            .update(&window.bytes[..used.min(window.bytes.len())]);
        tracked.lines = lines;
        tracked.identity = identity;
        let more = consumed > 0 && tracked.lines.cursor() < identity.len;
        if let Some(stream) = self.stream(&place) {
            // Canonical retention (B2 F2 ownership boundary): each distinct
            // event once, at exact capacity, so what the reader keeps is
            // what the fold charges. The held events are one sorted run and
            // the chunk's new events a second, disjoint from the first; the
            // stable sort merges the runs.
            stream.events = held;
            stream.events.extend(gathered);
            stream.events.sort();
            stream.events.dedup();
            stream.events.shrink_to_fit();
            stream.malformed += malformed;
            stream.over_cap += over_cap;
        }
        self.reweigh(&place.conversation);
        self.enforce();
        if !self.files.contains_key(&place) {
            // Refused by this chunk's growth: evicted, read no more.
            return;
        }
        if more {
            self.queue(Task::Read {
                place,
                anchor: Some(identity),
            });
        } else {
            self.finish(&place, true);
        }
    }

    /// A file's reading ended this round; `whole` when it reached the end.
    fn finish(&mut self, place: &Place, whole: bool) {
        self.recounting.remove(place);
        if let Some(tracked) = self.files.get_mut(place) {
            tracked.done = whole;
        }
        let settled = self.settled(&place.conversation);
        if let Some(conversation) = self.conversation(place) {
            if !whole && !conversation.gaps.contains(&GapKind::Changed) {
                conversation.gaps.push(GapKind::Changed);
                conversation.gaps.sort();
            }
            conversation.read = settled
                && !conversation.gaps.contains(&GapKind::Unlocated)
                && !conversation.gaps.contains(&GapKind::Changed);
        }
    }
}

/// A fresh reader run through one whole round.
pub(crate) fn scan(state_root: PathBuf, home: Option<PathBuf>, caps: Caps) -> Reader {
    let mut reader = Reader::new(state_root, home, caps);
    reader.round();
    while reader.step().is_some() {}
    reader
}

/// Every unit a [`Reader::step`] completed on this thread since the last
/// call, as its door counters recorded it: the A8 observation hook.
#[cfg(test)]
#[must_use]
pub(crate) fn take_units() -> Vec<Unit> {
    observe::UNITS.with(std::cell::RefCell::take)
}

/// Keep `unit` for [`take_units`] (tests only).
#[cfg(test)]
fn observed(unit: Unit) {
    observe::UNITS.with(|units| units.borrow_mut().push(unit));
}

/// Production keeps no unit record.
#[cfg(not(test))]
const fn observed(_: Unit) {}

/// The test-only record of completed units, filled at the end of each step.
#[cfg(test)]
mod observe {
    thread_local! {
        pub(super) static UNITS: std::cell::RefCell<Vec<super::Unit>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
}

#[cfg(test)]
#[path = "calls_read_spec.rs"]
mod spec;

#[cfg(test)]
#[path = "calls_persist_spec.rs"]
mod persist_spec;

#[cfg(test)]
#[path = "calls_read_tests.rs"]
mod tests;
