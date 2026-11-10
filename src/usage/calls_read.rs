//! The tool-statistics reader: association discovery and stepped, validated
//! transcript reads at the usage door, persisted under `<root>/stats/tools/`
//! (plan-r3 §5, §6; plan r4 §2-§5).
//!
//! One [`Reader`] per process, owned by the upkeep thread. A round loads what
//! the store holds (once per payload epoch), lists the ae-associated
//! conversations (live seats and their priors, `retire` records, kept
//! archives' `harness.tsv`), locates each one's Claude or Codex files, and
//! reads only what is new, in units: one [`Reader::step`] does ONE unit of
//! bounded IO and returns what the doors did ([`Unit`]).
//!
//! Every conversation is one [`Conv`]: its counters and its resume state, one
//! document under `stats/tools/conv/`. A file whose status (dev, ino, len,
//! mtime, ctime) is unchanged reads no byte. A grown file re-hashes its last
//! witness block (1 MiB) and resumes at its cursor; a same-length change
//! re-hashes every block; any mismatch, a new identity or a truncation below
//! the cursor rebuilds the conversation from 0. Association sources are
//! cached the same way (`stats/tools/sources`): an unchanged meta, journal or
//! archive record is not read again, a grown journal resumes at its cursor.
//! The cache keeps the roots ae itself recorded for each seat (lead ruling
//! Q1 = A), never a path a transcript named.
//!
//! Publication runs through [`disk::tools`] under `stats.lock` in a proven S5
//! epoch: Tools never makes the payload. Before an epoch exists, or for a
//! cold [`scan`], everything stays in memory.
//!
//! Every transcript, journal and meta read goes through a usage door, and a
//! [`Unit`]'s counters are the actual receipt of its door calls
//! ([`super::receipt`]); store IO is counted at the document reads and
//! writes.
use super::lines::{Framed, LINE_CAP, Lines};
use super::receipt::{Counts, Guard};
use super::{Entry, claude_calls, codex_calls};
use crate::meta::{RecordedClient, RecordedConfigHome, RecordedWorkDir, RosterEntry};
use crate::quota::{Bounded, Budget, DayListings};
use crate::stats_store::disk;
use crate::tool::{ToolKind, UsageSource};
use crate::tool_stats::doc::{self, Seat, SourceRecord, Sources};
use crate::tool_stats::incr::{
    self, Association, Conv, Coverage, FileState, Memberships, Published, PublishedConv, Source,
    StreamKey,
};
use crate::tool_stats::store::{self, Loaded, StoreError};
use crate::tool_stats::{
    Adapter, Caps, Conversation, Event, GapKind, Membership, Parsed, Role, StreamRole,
};
#[cfg(test)]
use crate::tool_stats::{Ask, View};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The most bytes one unit reads.
pub(crate) const STEP: u64 = 4 << 20;
/// The most lstats one unit makes.
pub(crate) const LSTATS: u32 = 256;
/// An archive's `harness.tsv` cap (the archive module's own byte cap).
const HARNESS_MAX: u64 = 1 << 20;
/// The longest membership field (session id, slot, client label) the reader
/// keeps. Session metas are hostile input: a membership with a longer field
/// is not kept and counts in a visible gap.
const FIELD_MAX: usize = doc::FIELD_MAX;
/// One witness block: SHA-256 per this many bytes of a consumed prefix.
const BLOCK: u64 = 1 << 20;
/// How long one publication waits for `stats.lock`.
const LOCK_WAIT: Duration = Duration::from_millis(50);

/// What one unit was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnitKind {
    /// A session meta status or read.
    Meta,
    /// One journal status, witness or chunk of a session's retire/reseat scan.
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
    /// Transcript, journal and meta bytes read.
    pub(crate) bytes: u64,
    /// Bytes of `stats/tools/` read or written.
    pub(crate) store_bytes: u64,
    pub(crate) lstats: u32,
    pub(crate) listings: u32,
    pub(crate) opens: u32,
}

impl Unit {
    fn of(kind: UnitKind, counts: Counts, store_bytes: u64) -> Self {
        let narrow = |count: u64| u32::try_from(count).unwrap_or(u32::MAX);
        Self {
            kind,
            bytes: counts.bytes,
            store_bytes,
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

/// Where a located file's observations go: its conversation and stream.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Place {
    conversation: Key,
    stream: StreamKey,
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

fn stamp((seconds, nanos): (i64, i64)) -> (i64, u32) {
    (seconds, u32::try_from(nanos).unwrap_or(0))
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

    fn state(&self, cursor: u64, skipping: bool, blocks: Vec<[u8; 32]>) -> FileState {
        FileState {
            dev: self.dev,
            ino: self.ino,
            len: self.len,
            mtime: stamp(self.mtime),
            ctime: stamp(self.ctime),
            cursor,
            skipping,
            blocks,
        }
    }

    /// The status `state` recorded, unchanged.
    fn same(&self, state: &FileState) -> bool {
        self.dev == state.dev
            && self.ino == state.ino
            && self.len == state.len
            && stamp(self.mtime) == state.mtime
            && stamp(self.ctime) == state.ctime
    }

    /// Another file, or one cut below what was consumed of it.
    const fn replaced(&self, state: &FileState) -> bool {
        self.dev != state.dev || self.ino != state.ino || self.len < state.cursor
    }
}

fn finish(context: ring::digest::Context) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out.copy_from_slice(context.finish().as_ref());
    out
}

fn sha() -> ring::digest::Context {
    ring::digest::Context::new(&ring::digest::SHA256)
}

/// The block witness of a consumed prefix, built forward.
#[derive(Clone)]
struct Witness {
    /// Every full block's digest.
    blocks: Vec<[u8; 32]>,
    /// The open block's hash and its bytes so far.
    open: ring::digest::Context,
    filled: u64,
}

impl Witness {
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            open: sha(),
            filled: 0,
        }
    }

    fn feed(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let room = usize::try_from(BLOCK - self.filled).unwrap_or(usize::MAX);
            let (now, rest) = bytes.split_at(room.min(bytes.len()));
            self.open.update(now);
            self.filled += u64::try_from(now.len()).unwrap_or(u64::MAX);
            bytes = rest;
            if self.filled == BLOCK {
                let full = std::mem::replace(&mut self.open, sha());
                self.blocks.push(finish(full));
                self.filled = 0;
            }
        }
    }

    /// The persisted form: every full block, then the open one if partial.
    fn blocks(&self) -> Vec<[u8; 32]> {
        let mut all = self.blocks.clone();
        if self.filled > 0 {
            all.push(finish(self.open.clone()));
        }
        all
    }
}

/// A re-hash of a consumed prefix against its recorded blocks, from block
/// `index` on.
struct Check {
    expect: Vec<[u8; 32]>,
    index: usize,
    at: u64,
    end: u64,
    witness: Witness,
}

impl Check {
    /// The check of `state`'s prefix: every block when `all`, else the last
    /// one only. `None` when its block list cannot be that prefix's.
    fn of(state: &FileState, all: bool) -> Option<Self> {
        let cursor = state.cursor;
        let last = cursor.checked_sub(1)? / BLOCK;
        let count = usize::try_from(last).ok()?.checked_add(1)?;
        if state.blocks.len() != count {
            return None;
        }
        let index = if all { 0 } else { count - 1 };
        let mut witness = Witness::new();
        witness.blocks = state.blocks.get(..index)?.to_vec();
        Some(Self {
            expect: state.blocks.clone(),
            index,
            at: u64::try_from(index).ok()? * BLOCK,
            end: cursor,
            witness,
        })
    }

    /// Feed the next bytes of the prefix; `false` on a mismatch.
    fn feed(&mut self, bytes: &[u8]) -> bool {
        let before = self.witness.blocks.len();
        self.witness.feed(bytes);
        self.at += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let done = &self.witness.blocks[before..];
        for (offset, block) in done.iter().enumerate() {
            if self.expect.get(before + offset) != Some(block) {
                return false;
            }
        }
        self.index = self.witness.blocks.len();
        true
    }

    /// At the end: whether the open partial block matches too.
    fn closes(&self) -> bool {
        self.witness.blocks() == self.expect
    }
}

/// The seat roots a Claude conversation's skill directories classify by.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Roots {
    config_homes: Vec<PathBuf>,
    work_dirs: Vec<PathBuf>,
}

/// One conversation's association this round.
#[derive(Debug, Default)]
struct Assoc {
    memberships: Vec<Membership>,
    /// The session groups and archives whose evidence it rests on.
    groups: BTreeSet<String>,
    /// Each store holding a copy.
    stores: BTreeSet<PathBuf>,
    roots: Roots,
}

/// Unsupported seats' conversations: per (normalized tool, conversation id),
/// the sessions holding a membership of it.
type Unsupported = BTreeMap<(String, String), BTreeSet<Option<String>>>;

/// A journal scan under way.
struct Scan {
    key: String,
    lines: Lines,
    witness: Witness,
    /// A witness check before the scan resumes.
    check: Option<Check>,
    record: SourceRecord,
    damaged: bool,
    /// The status the scan's first window saw: every later window, check or
    /// chunk, must see it too, or the scan ends without mixing evidence.
    anchor: Option<Identity>,
}

impl Scan {
    /// Whether `window` saw the scan's status (the first window sets it).
    fn anchored(&mut self, window: &super::JournalWindow) -> bool {
        let identity = Identity::window(window);
        *self.anchor.get_or_insert(identity) == identity
    }
}

/// One unit of work.
enum Task {
    Load,
    Recover,
    /// The store documents again, after a recovery moved them.
    Reload,
    Sessions,
    Meta {
        dir: PathBuf,
        group: String,
    },
    Journal {
        dir: PathBuf,
        group: String,
        session: Option<String>,
        scan: Option<Box<Scan>>,
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
    LoadConv(Key),
    Lstats(Vec<(Place, PathBuf)>),
    Verify {
        place: Place,
        generation: u64,
    },
    Read {
        place: Place,
        generation: u64,
    },
    Publish(Key),
    PublishView,
    PublishSources,
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
            Self::Load | Self::Reload | Self::LoadConv(_) => UnitKind::Load,
            Self::Publish(_) | Self::PublishView | Self::PublishSources => UnitKind::Publish,
            Self::Recover => UnitKind::Recover,
        }
    }
}

/// Where a round is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Load,
    Associate,
    Locate,
    Check,
    Publish,
}

/// A file's reading this round.
enum Stage {
    /// Re-hashing the consumed prefix before resuming.
    Verifying(Box<Verifying>),
    Reading(Box<Reading>),
}

/// A consumed prefix being re-hashed.
struct Verifying {
    path: PathBuf,
    anchor: Identity,
    check: Check,
    state: FileState,
}

/// New bytes of one file being read.
struct Reading {
    path: PathBuf,
    roots: Roots,
    lines: Lines,
    witness: Witness,
    /// The status every chunk must still see.
    anchor: Identity,
    /// The status of the last chunk taken.
    identity: Identity,
    events: Vec<Event>,
    malformed: u64,
    over_cap: u64,
    /// Bytes were taken: the file's state moves at settlement.
    took: bool,
    done: bool,
    /// Read from 0: once taken, it replaces what the stream held.
    fresh: bool,
}

/// One file's stage and the generation its queued tasks carry.
struct Work {
    generation: u64,
    stage: Stage,
}

/// One round's discovery and reading state.
struct Round {
    phase: Phase,
    tasks: VecDeque<Task>,
    /// This round's association records, by source key.
    fresh: Sources,
    /// Session groups and archives whose evidence was unavailable.
    incomplete: BTreeSet<String>,
    sessions_listed: bool,
    archives_listed: bool,
    assoc: BTreeMap<Key, Assoc>,
    reseated: BTreeSet<String>,
    unsupported: Unsupported,
    oversize: u64,
    /// Claude: (store, id) → every project file `<id>.jsonl` found.
    parents: BTreeMap<(PathBuf, String), Vec<PathBuf>>,
    /// Claude: (store, id) → its `subagents/` files, by stem.
    children: BTreeMap<(PathBuf, String), BTreeMap<String, PathBuf>>,
    /// Stores or roots whose listing failed or was cut.
    broken: BTreeSet<PathBuf>,
    days: DayListings,
    listed_days: BTreeSet<PathBuf>,
    rollouts: BTreeMap<(PathBuf, String), Result<Option<PathBuf>, GapKind>>,
    located: BTreeMap<Place, PathBuf>,
    with_parent: BTreeSet<Key>,
    marks: BTreeMap<Key, Vec<GapKind>>,
    /// Per conversation, its located streams not settled yet this round.
    open: BTreeMap<Key, BTreeSet<StreamKey>>,
    work: BTreeMap<Place, Work>,
    /// Streams that changed under a read, or vanished, this round.
    changed: BTreeSet<Place>,
    missing: BTreeSet<Place>,
    /// Streams whose own lstat this round found them on disk.
    statted: BTreeSet<Place>,
    /// Conversations rebuilt this round.
    rebuilt: BTreeSet<Key>,
}

impl Round {
    fn new(load: bool) -> Self {
        let (phase, tasks) = if load {
            (Phase::Load, VecDeque::from([Task::Load]))
        } else {
            (
                Phase::Associate,
                VecDeque::from([Task::Sessions, Task::Archives]),
            )
        };
        Self {
            phase,
            tasks,
            fresh: Sources::new(),
            incomplete: BTreeSet::new(),
            sessions_listed: true,
            archives_listed: true,
            assoc: BTreeMap::new(),
            reseated: BTreeSet::new(),
            unsupported: Unsupported::new(),
            oversize: 0,
            parents: BTreeMap::new(),
            children: BTreeMap::new(),
            broken: BTreeSet::new(),
            days: DayListings::default(),
            listed_days: BTreeSet::new(),
            rollouts: BTreeMap::new(),
            located: BTreeMap::new(),
            with_parent: BTreeSet::new(),
            marks: BTreeMap::new(),
            open: BTreeMap::new(),
            work: BTreeMap::new(),
            changed: BTreeSet::new(),
            missing: BTreeSet::new(),
            statted: BTreeSet::new(),
            rebuilt: BTreeSet::new(),
        }
    }
}

/// The association cache and the view await publication.
#[derive(Debug, Clone, Copy)]
struct Dirty {
    sources: bool,
    view: bool,
}

/// Where the held state stands against the store's epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proof {
    /// Nothing loaded yet.
    Cold,
    /// Loaded, and this round's epoch read proved the held epoch.
    Proven,
    /// Loaded, but the last epoch read failed: nothing is published.
    Lost,
}

/// One conversation the reader holds.
struct Held {
    conv: Conv,
    /// Changed since its last durable publication.
    dirty: bool,
    /// Held from its view entry (summary and head, no evidence): the
    /// document is loaded before anything changes it (plan r4 section 1).
    partial: bool,
}

/// The reader's whole state.
pub(crate) struct Reader {
    state_root: PathBuf,
    home: Option<PathBuf>,
    caps: Caps,
    /// Publishes to `<root>/stats/tools/`; a cold [`scan`] does not.
    durable: bool,
    /// Whether what the store held was loaded, and whether this round's
    /// epoch read proved `epoch`: only then is anything published.
    proof: Proof,
    /// The epoch the held state belongs to (`None`: no payload yet): kept
    /// while a proof fails, so a payload proven again under another epoch
    /// still drops it.
    epoch: Option<u64>,
    /// The association cache could not be read: it is never replaced until
    /// a later read answers.
    sources_blocked: bool,
    /// The last view generation loaded or published.
    generation: u64,
    convs: BTreeMap<Key, Held>,
    /// The association cache as last published (or loaded).
    sources: Sources,
    /// What awaits publication besides the conversations.
    dirty: Dirty,
    /// Every associated conversation, in `(adapter, id)` order.
    all: Published,
    /// Each one's part of a rendered view, aligned with `all.convs`.
    parts: Vec<(u64, u64)>,
    association: Memberships,
    conversations: Vec<Conversation>,
    unsupported: Unsupported,
    /// Association sources whose evidence was unavailable last round.
    hidden: usize,
    oversize: u64,
    /// Files being read again from 0 this round.
    recounting: u64,
    /// Generation of the next file stage.
    next: u64,
    round: Option<Round>,
    /// The coverage last published beside the view (its generation 0).
    covered: Option<Coverage>,
}

impl std::fmt::Debug for Reader {
    /// Counts only: the per-file state holds paths, which never leave it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader")
            .field("conversations", &self.convs.len())
            .field("busy", &self.busy())
            .finish_non_exhaustive()
    }
}

/// A [`disk::Dir`] that counts the document bytes read through it.
struct Counted<'a> {
    dir: &'a mut dyn disk::Dir,
    bytes: u64,
}

impl disk::Dir for Counted<'_> {
    fn read(&mut self, name: &str, cap: u64) -> disk::Read {
        let read = self.dir.read(name, cap);
        if let disk::Read::Bytes(bytes) = &read {
            self.bytes = self
                .bytes
                .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        }
        read
    }

    fn apply(&mut self, op: &disk::Op) -> std::io::Result<()> {
        self.dir.apply(op)
    }

    fn list(&mut self, dir: &str) -> std::io::Result<Vec<String>> {
        self.dir.list(dir)
    }
}

/// The bytes a plan writes.
fn written(plan: &disk::Plan) -> u64 {
    plan.ops
        .iter()
        .map(|op| match op {
            disk::Op::Write { bytes, .. } => u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            _ => 0,
        })
        .fold(0, u64::saturating_add)
}

/// How one publication attempt ended.
enum Outcome {
    /// The held state belongs to another payload epoch.
    Moved,
    Recovered(u64),
    Published(u64),
    Stale,
    Failed,
}

impl Reader {
    /// A reader of the ae state root `state_root`; `home` resolves an implicit
    /// harness store. Nothing is read until [`Self::round`].
    pub(crate) const fn new(state_root: PathBuf, home: Option<PathBuf>, caps: Caps) -> Self {
        Self::with(state_root, home, caps, true)
    }

    const fn with(state_root: PathBuf, home: Option<PathBuf>, caps: Caps, durable: bool) -> Self {
        Self {
            state_root,
            home,
            caps,
            durable,
            proof: Proof::Cold,
            epoch: None,
            sources_blocked: false,
            generation: 0,
            convs: BTreeMap::new(),
            sources: Sources::new(),
            dirty: Dirty {
                sources: false,
                view: false,
            },
            all: Published {
                generation: 0,
                convs: Vec::new(),
                over_cap: 0,
            },
            parts: Vec::new(),
            association: Memberships::new(),
            conversations: Vec::new(),
            unsupported: Unsupported::new(),
            hidden: 0,
            oversize: 0,
            recounting: 0,
            next: 0,
            round: None,
            covered: None,
        }
    }

    /// Start a discovery round (on show, then every upkeep period); a round
    /// in progress continues.
    pub(crate) fn round(&mut self) {
        if self.round.is_none() {
            self.round = Some(Round::new(self.durable));
        }
    }

    /// Do ONE unit of the round; `None` when the round is done.
    pub(crate) fn step(&mut self) -> Option<Unit> {
        loop {
            let mut round = self.round.take()?;
            let Some(task) = round.tasks.pop_front() else {
                if self.advance(&mut round) {
                    self.round = Some(round);
                    continue;
                }
                self.recounting = 0;
                return None;
            };
            // The unit's receipt is what the doors actually did on this
            // thread between its start and its end.
            let kind = task.kind();
            let guard = Guard::start();
            let store = self.run(&mut round, task, &guard);
            let unit = Unit::of(kind, guard.delta(), store);
            if self.round.is_none() {
                self.round = Some(round);
            }
            observed(unit);
            return Some(unit);
        }
    }

    /// Whether a round is in progress.
    pub(crate) const fn busy(&self) -> bool {
        self.round.is_some()
    }

    /// Whether the last load proved an active S5 epoch to publish under.
    pub(crate) const fn proven(&self) -> bool {
        self.publishing()
    }

    /// End the round in progress before its next unit: the upkeep's answer
    /// to a load that proved no epoch, before any discovery.
    pub(crate) fn abandon(&mut self) {
        self.round = None;
        self.recounting = 0;
    }

    /// The page `ask` over the published view, with the coverage lines and
    /// the recount count: what a loader draws from `tools/view` and
    /// `tools/coverage` of one generation.
    /// The test oracle: production draws from the store (`app::loader`).
    #[cfg(test)]
    pub(crate) fn view(&self, ask: &Ask) -> View {
        // The prefix the view of this state is published with: the held
        // generation, or the next one while a change awaits publication.
        let generation = self
            .generation
            .saturating_add(u64::from(self.dirty.view || self.generation == 0));
        let kept = self.fitted(generation);
        let left = u64::try_from(self.all.convs.len() - kept).unwrap_or(u64::MAX);
        let mut view = incr::combine_parts(&self.all.convs[..kept], left, ask, &self.caps);
        self.coverage(self.generation).apply(&mut view, ask);
        view
    }

    /// The coverage beside the view of `generation` (lead ruling Q2 = A), as
    /// published: under the coverage cap, any record left out counted in a
    /// visible over-cap line.
    fn coverage(&self, generation: u64) -> Coverage {
        let whole = Coverage {
            generation,
            ..self.unfitted()
        };
        doc::fit_coverage(&whole, self.caps.coverage_cap)
    }

    /// What the coverage says before its cap, at generation 0: what a
    /// publication is compared by.
    fn unfitted(&self) -> Coverage {
        let mut unsupported: Vec<(String, BTreeSet<Option<String>>)> = self
            .unsupported
            .iter()
            .map(|((tool, _), sessions)| (tool.clone(), sessions.clone()))
            .collect();
        unsupported.sort();
        Coverage {
            generation: 0,
            hidden: u64::try_from(self.hidden).unwrap_or(u64::MAX),
            oversize: self.oversize,
            recounting: self.recounting,
            unsupported,
            omitted: 0,
        }
    }

    /// The association records: memberships, flags and discovery gaps; no
    /// observation (the privacy surface besides the view).
    #[cfg(test)]
    pub(crate) fn conversations(&self) -> &[Conversation] {
        &self.conversations
    }

    /// Run one task; returns the store bytes it read or wrote.
    fn run(&mut self, round: &mut Round, task: Task, guard: &Guard) -> u64 {
        match task {
            Task::Load => return self.load(round),
            Task::Recover => return self.recover(round),
            Task::Reload => return self.reload(),
            Task::Sessions => self.sessions(round),
            Task::Meta { dir, group } => self.meta(round, &dir, group),
            Task::Journal {
                dir,
                group,
                session,
                scan,
            } => self.journal(round, dir, group, session, scan),
            Task::Archives => self.archives(round),
            Task::Archive { dir, id } => self.archive(round, &dir, &id),
            Task::ClaudeStore { store } => claude_store(round, store),
            Task::ClaudeProject { store, project } => claude_project(round, store, &project),
            Task::Subagents { store, id, dir } => subagents(round, store, id, &dir),
            Task::CodexDay { dir } => codex_day(round, &dir),
            Task::CodexFind { root, id } => codex_find(round, root, id),
            Task::LoadConv(key) => return self.load_conv(round, &key),
            Task::Lstats(batch) => self.lstats(round, batch, guard),
            Task::Verify { place, generation } => self.verify(round, place, generation),
            Task::Read { place, generation } => self.read(round, place, generation),
            Task::Publish(key) => return self.publish(round, &key),
            Task::PublishView => return self.publish_view(round),
            Task::PublishSources => return self.publish_sources(round),
        }
        0
    }

    /// Move to the next phase when the queue ran dry; `false` when the round
    /// is over.
    fn advance(&mut self, round: &mut Round) -> bool {
        match round.phase {
            Phase::Load => {
                round.phase = Phase::Associate;
                round.tasks.extend([Task::Sessions, Task::Archives]);
                true
            }
            Phase::Associate => {
                round.phase = Phase::Locate;
                self.associate(round);
                queue_locate(round);
                true
            }
            Phase::Locate => {
                round.phase = Phase::Check;
                self.place(round);
                true
            }
            Phase::Check => {
                round.phase = Phase::Publish;
                // Every read of the round is settled: nothing is being recounted,
                // and a coverage that moved is published with the view.
                self.recounting = 0;
                if self.covered.as_ref() != Some(&self.unfitted()) {
                    self.dirty.view = true;
                }
                if self.publishing() {
                    if self.dirty.sources {
                        round.tasks.push_back(Task::PublishSources);
                    }
                    if self.dirty.view {
                        round.tasks.push_back(Task::PublishView);
                    }
                }
                true
            }
            Phase::Publish => false,
        }
    }

    /// Whether publications are made: a durable reader in a proven epoch.
    const fn publishing(&self) -> bool {
        self.durable && matches!(self.proof, Proof::Proven) && self.epoch.is_some()
    }
}

// --- The store (plan r4 §2) ------------------------------------------------------

impl Reader {
    /// Each round: the active epoch, read without the lock. Another epoch than
    /// the held state's (a reset `r`, a repair) drops it, and what the new
    /// payload holds is loaded.
    fn load(&mut self, round: &mut Round) -> u64 {
        let mut files = disk::Files::at(&self.state_root);
        let mut dir = Counted {
            dir: &mut files,
            bytes: 0,
        };
        let Ok(epoch) = disk::active_epoch(&mut dir) else {
            // Damage S5 repairs: nothing is published until it does. The
            // held epoch stays, so a repair under another epoch drops the
            // held state at the next proof.
            self.lose_proof();
            return dir.bytes;
        };
        if self.proof != Proof::Cold && self.epoch.is_some() && epoch != self.epoch {
            self.forget();
        }
        if self.proof != Proof::Cold {
            self.epoch = epoch;
            self.proof = Proof::Proven;
            if self.sources_blocked && epoch.is_some() {
                // Retried until the cache answers; only then may it be replaced.
                self.sources_blocked =
                    matches!(store::load_sources(&mut dir), Loaded::Unreadable(_));
            }
            return dir.bytes;
        }
        self.epoch = epoch;
        self.proof = Proof::Proven;
        if epoch.is_none() {
            return dir.bytes;
        }
        self.documents(&mut dir);
        round.tasks.push_back(Task::Recover);
        dir.bytes
    }

    /// Read the view, its coverage, the sources and the memberships: what a
    /// load holds, and what a recovery that moved the store holds again.
    fn documents(&mut self, dir: &mut dyn disk::Dir) {
        self.covered = None;
        if let Loaded::Ok(view) = store::load_view(dir) {
            self.generation = view.generation;
            // The published entries are the held view: a conversation that
            // settles to its own entry again publishes nothing, so a reopened
            // reader does not move the generation under another writer.
            self.parts = view.convs.iter().map(doc::view_part).collect();
            self.all = Published {
                generation: view.generation,
                convs: view.convs.clone(),
                over_cap: 0,
            };
            // The coverage published beside that view needs no new publication.
            if let Loaded::Ok(coverage) = store::load_coverage(dir)
                && coverage.generation == view.generation
            {
                self.covered = Some(Coverage {
                    generation: 0,
                    ..coverage
                });
            }
        }
        match store::load_sources(dir) {
            Loaded::Ok(sources) => self.sources = sources,
            Loaded::Absent | Loaded::Rebuild(_) => self.sources = Sources::new(),
            Loaded::Unreadable(_) => {
                // An IO refusal is not an empty cache: everything is read
                // again, and the document is not replaced.
                self.sources = Sources::new();
                self.sources_blocked = true;
            }
        }
        // The memberships published beside the sources: a round that finds
        // the same ones publishes no association (P5: no publication
        // without a change).
        if let Loaded::Ok(assoc) = store::load_assoc(dir) {
            self.association = assoc.memberships;
        }
    }

    /// The epoch could not be proven: nothing is published until it is.
    fn lose_proof(&mut self) {
        if self.proof != Proof::Cold {
            self.proof = Proof::Lost;
        }
    }

    /// Drop everything held: the payload under it was replaced.
    fn forget(&mut self) {
        let fresh = Self::with(
            self.state_root.clone(),
            self.home.clone(),
            self.caps,
            self.durable,
        );
        *self = fresh;
    }

    /// Finish or undo an interrupted publication before any other.
    fn recover(&mut self, round: &mut Round) -> u64 {
        let held = self.epoch;
        let outcome = disk::tools(&self.state_root, LOCK_WAIT, |dir, epoch| {
            if held != Some(epoch) {
                return (None, None);
            }
            match store::recovery(dir) {
                Ok(plan) if !plan.ops.is_empty() => {
                    let bytes = written(&plan);
                    (Some(plan), Some(bytes))
                }
                Ok(_) | Err(_) => (None, None),
            }
        });
        match outcome {
            disk::Tools::Done {
                value: Some(bytes), ..
            } => {
                // The store moved under the loaded documents (a committed
                // view renamed in, or a staged one undone): they are read
                // again next, so no publication starts from an older
                // generation.
                round.tasks.push_front(Task::Reload);
                bytes
            }
            _ => 0,
        }
    }

    /// Read the store documents again (see [`Self::recover`]).
    fn reload(&mut self) -> u64 {
        let mut files = disk::Files::at(&self.state_root);
        let mut dir = Counted {
            dir: &mut files,
            bytes: 0,
        };
        self.documents(&mut dir);
        dir.bytes
    }

    /// One conversation's durable document, or a fresh one to build.
    fn load_conv(&mut self, round: &mut Round, key: &Key) -> u64 {
        let mut files = disk::Files::at(&self.state_root);
        let mut dir = Counted {
            dir: &mut files,
            bytes: 0,
        };
        // A mark set here is published even when no count moves.
        let (conv, dirty) = match store::load_conv(&mut dir, key.0, &key.1) {
            Loaded::Ok(conv) => (conv, false),
            Loaded::Absent => (Conv::new(key.0, &key.1), false),
            // Damaged, oversized or of another format: rebuilt from 0, and
            // marked: counts of its deleted sources are lost (P6 boundary).
            Loaded::Rebuild(_) => {
                let mut conv = Conv::new(key.0, &key.1);
                conv.mark_rebuilt();
                (conv, true)
            }
            Loaded::Unreadable(_) => {
                // Never rebuilt over an IO error: not read this round.
                let streams: Vec<StreamKey> = round
                    .open
                    .get(key)
                    .map(|open| open.iter().cloned().collect())
                    .unwrap_or_default();
                for stream in streams {
                    round.changed.insert(Place {
                        conversation: key.clone(),
                        stream,
                    });
                }
                round.open.remove(key);
                // Not held from its view entry either: not read this round.
                if self.convs.get(key).is_some_and(|held| held.partial) {
                    self.convs.remove(key);
                }
                return dir.bytes;
            }
        };
        let bytes = dir.bytes;
        self.convs.insert(
            key.clone(),
            Held {
                conv,
                dirty,
                partial: false,
            },
        );
        if round.open.get(key).is_some_and(BTreeSet::is_empty) {
            self.settle(round, key);
        }
        bytes
    }

    /// Publish one conversation and the view.
    fn publish(&mut self, round: &mut Round, key: &Key) -> u64 {
        if !self.publishing() {
            return 0;
        }
        let Some(held) = self.convs.get(key) else {
            return 0;
        };
        if !held.dirty {
            return 0;
        }
        let generation = self.generation.saturating_add(1);
        let view = self.capped(generation);
        let coverage = self.coverage(generation);
        let conv = &held.conv;
        let epoch = self.epoch;
        let outcome = disk::tools(&self.state_root, LOCK_WAIT, |dir, now| {
            if epoch != Some(now) {
                return (None, Outcome::Moved);
            }
            let mut dir = Counted { dir, bytes: 0 };
            match store::recovery(&mut dir) {
                Ok(plan) if !plan.ops.is_empty() => {
                    let bytes = written(&plan).saturating_add(dir.bytes);
                    return (Some(plan), Outcome::Recovered(bytes));
                }
                Ok(_) => {}
                Err(_) => return (None, Outcome::Failed),
            }
            match store::publish_with(&mut dir, conv, &view, &coverage) {
                Ok(plan) => {
                    let bytes = written(&plan).saturating_add(dir.bytes);
                    (Some(plan), Outcome::Published(bytes))
                }
                Err(StoreError::Stale) => (None, Outcome::Stale),
                Err(StoreError::Io(_)) => (None, Outcome::Failed),
            }
        });
        self.after(round, outcome, Some(key), generation)
    }

    /// Publish the view alone: the association moved without a count.
    fn publish_view(&mut self, round: &mut Round) -> u64 {
        if !self.publishing() || !self.dirty.view {
            return 0;
        }
        let generation = self.generation.saturating_add(1);
        let view = self.capped(generation);
        let coverage = self.coverage(generation);
        let epoch = self.epoch;
        let outcome = disk::tools(&self.state_root, LOCK_WAIT, |dir, now| {
            if epoch != Some(now) {
                return (None, Outcome::Moved);
            }
            let mut dir = Counted { dir, bytes: 0 };
            match store::publish_view_with(&mut dir, &view, &coverage) {
                Ok(plan) => {
                    let bytes = written(&plan).saturating_add(dir.bytes);
                    (Some(plan), Outcome::Published(bytes))
                }
                Err(StoreError::Stale) => (None, Outcome::Stale),
                Err(StoreError::Io(_)) => (None, Outcome::Failed),
            }
        });
        self.after(round, outcome, None, generation)
    }

    /// Publish the association cache and its derived document.
    fn publish_sources(&mut self, round: &mut Round) -> u64 {
        if !self.publishing() || !self.dirty.sources || self.sources_blocked {
            return 0;
        }
        // A cache that would not read back is never written: its sources
        // are read again by the next process instead.
        let sources = self.sources.clone();
        if doc::parse_sources(&doc::render_sources(&sources)).as_ref() != Ok(&sources) {
            self.dirty.sources = false;
            return 0;
        }
        let assoc = doc::Assoc {
            sources: sources
                .iter()
                .map(|(key, record)| (key.clone(), record.status.clone()))
                .collect(),
            memberships: self.association.clone(),
        };
        let epoch = self.epoch;
        let outcome = disk::tools(&self.state_root, LOCK_WAIT, |dir, now| {
            if epoch != Some(now) {
                return (None, Outcome::Moved);
            }
            match store::publish_assoc(dir, &sources, &assoc) {
                Ok(plan) => {
                    let bytes = written(&plan);
                    (Some(plan), Outcome::Published(bytes))
                }
                Err(_) => (None, Outcome::Failed),
            }
        });
        match outcome {
            disk::Tools::Done {
                value: Outcome::Published(bytes),
                ..
            } => {
                self.dirty.sources = false;
                bytes
            }
            disk::Tools::Done {
                value: Outcome::Moved,
                ..
            } => {
                self.forget();
                round.tasks.clear();
                0
            }
            _ => 0,
        }
    }

    /// What a publication attempt's outcome does to the held state.
    fn after(
        &mut self,
        round: &mut Round,
        outcome: disk::Tools<Outcome>,
        key: Option<&Key>,
        generation: u64,
    ) -> u64 {
        let retry = |round: &mut Round| {
            round
                .tasks
                .push_front(key.map_or(Task::PublishView, |key| Task::Publish(key.clone())));
        };
        match outcome {
            disk::Tools::Done { value, .. } => match value {
                Outcome::Published(bytes) => {
                    self.generation = generation;
                    self.dirty.view = false;
                    self.covered = Some(self.unfitted());
                    if let Some(held) = key.and_then(|key| self.convs.get_mut(key)) {
                        held.dirty = false;
                        // Processed and durable: only its view entry is kept.
                        release(held);
                    }
                    bytes
                }
                Outcome::Recovered(bytes) => {
                    retry(round);
                    bytes
                }
                // Another writer published since this state was loaded (every
                // publication moves the view generation, so the comparison
                // under the lock covers each conversation's revision too):
                // the held state may be older than the store's. It is dropped
                // and the next round loads the store's own.
                Outcome::Stale | Outcome::Moved => {
                    self.forget();
                    round.tasks.clear();
                    0
                }
                Outcome::Failed => 0,
            },
            disk::Tools::NoEpoch => {
                self.lose_proof();
                0
            }
            disk::Tools::Busy | disk::Tools::Failed(_) => 0,
        }
    }

    /// The view as published: the longest `(adapter, id)` prefix of every
    /// associated conversation whose document fits the view cap.
    fn capped(&self, generation: u64) -> Published {
        let kept = self.fitted(generation);
        Published {
            generation,
            convs: self.all.convs[..kept].to_vec(),
            over_cap: u64::try_from(self.all.convs.len() - kept).unwrap_or(u64::MAX),
        }
    }

    /// Replace `key`'s entry in the view by `conv` (none: drop it).
    ///
    /// # Errors
    ///
    /// [`incr::Overflow`] when the view could no longer be added up: it is
    /// kept as it was.
    fn enter(&mut self, key: &Key, conv: Option<PublishedConv>) -> Result<(), incr::Overflow> {
        let at = self.all.convs.binary_search_by(|one| {
            (one.summary.adapter, one.summary.id.as_str()).cmp(&(key.0, key.1.as_str()))
        });
        match (conv, at) {
            (Some(conv), Ok(index)) => {
                if self.all.convs[index] == conv {
                    return Ok(());
                }
                let part = doc::view_part(&conv);
                self.all.admit(conv)?;
                self.parts[index] = part;
            }
            (Some(conv), Err(index)) => {
                let part = doc::view_part(&conv);
                self.all.admit(conv)?;
                self.parts.insert(index, part);
            }
            (None, Ok(index)) => {
                self.all.convs.remove(index);
                self.parts.remove(index);
            }
            (None, Err(_)) => return Ok(()),
        }
        self.dirty.view = true;
        Ok(())
    }

    /// The longest prefix whose view, rendered with `generation`, fits the
    /// view cap: the generation's digits are part of the document.
    fn fitted(&self, generation: u64) -> usize {
        let count = self.parts.len();
        let mut bytes = 0_u64;
        let mut records = 0_u64;
        let mut kept = 0;
        for (index, (part, recs)) in self.parts.iter().enumerate() {
            let over = u64::try_from(count - index - 1).unwrap_or(u64::MAX);
            let total = bytes.saturating_add(*part).saturating_add(doc::view_frame(
                generation,
                over,
                records.saturating_add(*recs),
            ));
            if total > self.caps.view_cap {
                break;
            }
            bytes = bytes.saturating_add(*part);
            records = records.saturating_add(*recs);
            kept = index + 1;
        }
        kept
    }
}

// --- Association (plan §5; cached per source, Q1 = A) ----------------------------

/// The digest naming a session directory in the association cache.
fn session_group(name: &str) -> String {
    digest_hex(b"tools-session", name.as_bytes())
}

/// The persisted identity of a Claude child file: a digest of its native
/// file stem, never the stem (P2). Distinct stems stay distinct streams, and
/// the same stem resumes the same stream.
fn child_id(stem: &str) -> String {
    digest_hex(b"tools-child", stem.as_bytes())
}

/// The digest of the roots a Claude conversation's events are classified
/// by: each list counted, each root length-prefixed.
fn roots_digest(roots: &Roots) -> crate::tool_stats::Key {
    use std::os::unix::ffi::OsStrExt as _;
    let mut bytes = Vec::new();
    for list in [&roots.config_homes, &roots.work_dirs] {
        bytes.extend_from_slice(&u64::try_from(list.len()).unwrap_or(u64::MAX).to_le_bytes());
        for root in list {
            let root = root.as_os_str().as_bytes();
            bytes.extend_from_slice(&u64::try_from(root.len()).unwrap_or(u64::MAX).to_le_bytes());
            bytes.extend_from_slice(root);
        }
    }
    crate::tool_stats::Key::of(b"tools-roots", &bytes)
}

/// 32 lowercase hex digits of the 16-byte digest of `bytes` in `domain`.
fn digest_hex(domain: &[u8], bytes: &[u8]) -> String {
    let key = crate::tool_stats::Key::of(domain, bytes);
    let mut out = String::with_capacity(32);
    for byte in key.0 {
        out.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        out.push(char::from(b"0123456789abcdef"[usize::from(byte & 15)]));
    }
    out
}

/// The completeness group of a source key: a session's meta and journal
/// share one, an archive is its own.
fn group_of(key: &str) -> String {
    // Whole prefixes: the bare word of a session file is spelled in
    // src/store.rs alone (tests/it doors).
    key.strip_prefix("meta:")
        .or_else(|| key.strip_prefix("journal:"))
        .unwrap_or(key)
        .to_owned()
}

/// Whether a source's evidence is whole: a journal read to its end with no
/// damaged line and no open over-cap line.
fn whole(key: &str, record: &SourceRecord) -> bool {
    if key.starts_with("journal:") {
        record.complete && !record.status.skipping && record.status.cursor == record.status.len
    } else {
        record.complete
    }
}

/// The digest of a store root: a stream's copy.
fn copy_of(store: &Path) -> [u8; 16] {
    use std::os::unix::ffi::OsStrExt as _;
    crate::tool_stats::Key::of(b"tools-store", store.as_os_str().as_bytes()).0
}

fn utf8(path: &Path) -> Option<String> {
    path.to_str()
        .filter(|text| doc::valid_root(text))
        .map(str::to_owned)
}

/// A new empty record of a source's status.
fn record(status: FileState, session: Option<String>) -> SourceRecord {
    SourceRecord {
        status,
        complete: true,
        oversize: 0,
        session,
        seats: Vec::new(),
        unsupported: BTreeSet::new(),
        reseats: BTreeSet::new(),
    }
}

/// A membership with a field over [`FIELD_MAX`] is not kept.
fn over_limit(membership: &Membership) -> bool {
    membership
        .session
        .as_ref()
        .is_some_and(|session| session.len() > FIELD_MAX)
        || membership.slot.len() > FIELD_MAX
        || matches!(&membership.client, RecordedClient::Label(label) if label.len() > FIELD_MAX)
}

/// Add one membership of `id` held by `seat` to `record`, with the roots ae
/// recorded for the seat.
fn add(
    record: &mut SourceRecord,
    home: Option<&Path>,
    seat: &RosterEntry,
    id: &str,
    membership: Membership,
    roots: &Roots,
) {
    if over_limit(&membership) || membership.slot.is_empty() {
        record.oversize = record.oversize.saturating_add(1);
        return;
    }
    let Some(adapter) = adapter_of(seat.binary.as_deref()) else {
        let tool = unsupported_tool(seat.binary.as_deref());
        record
            .unsupported
            .insert((tool, id.to_owned(), membership.session));
        return;
    };
    let usage = match adapter {
        Adapter::Claude => UsageSource::ClaudeTranscripts,
        Adapter::Codex => UsageSource::CodexRollout,
    };
    // The store as the meta recorded it, resolved without IO; none leaves
    // the conversation listed and unlocated.
    let store = super::source_lexical(seat, usage, home).ok();
    let mut keep = |path: &PathBuf| {
        let text = utf8(path);
        if text.is_none() {
            // A root the cache cannot keep: the evidence is not whole.
            record.complete = false;
        }
        text
    };
    let store = store.as_ref().and_then(&mut keep);
    let config_homes = roots.config_homes.iter().filter_map(&mut keep).collect();
    let work_dirs = roots.work_dirs.iter().filter_map(&mut keep).collect();
    let seat = Seat {
        adapter,
        id: id.to_owned(),
        membership,
        store,
        config_homes,
        work_dirs,
    };
    if !record.seats.contains(&seat) {
        record.seats.push(seat);
    }
}

/// One journal record: `retire` adds a retired seat, `reseat` names the ids
/// it moved.
fn journal_record(
    record: &mut SourceRecord,
    home: Option<&Path>,
    event: &crate::events::Event,
    session: Option<&str>,
) {
    match event.action.as_str() {
        "reseat" => {
            record
                .reseats
                .extend(reseat_ids(event.summary.as_deref().unwrap_or("")));
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
            add(record, home, &seat, &id, membership, &roots);
        }
        _ => {}
    }
}

impl Reader {
    fn sessions(&mut self, round: &mut Round) {
        let root = self.state_root.join("sessions");
        match crate::quota::session_paths(&root, &mut Budget::local(u64::MAX)) {
            Ok(Bounded::Ready(paths)) => {
                for dir in paths {
                    let name = dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(session_group);
                    if let Some(group) = name {
                        round.tasks.push_back(Task::Meta { dir, group });
                    }
                }
            }
            Ok(Bounded::Truncated) | Err(_) => round.sessions_listed = false,
        }
    }

    fn meta(&mut self, round: &mut Round, dir: &Path, group: String) {
        let key = format!("meta:{group}");
        let path = dir.join(crate::meta::FILE);
        let mut budget = Budget::local(u64::MAX);
        let Ok(Some(file)) = super::locate_transcript(&path, &mut budget) else {
            round.incomplete.insert(group);
            return;
        };
        let identity = Identity::of(&file.metadata);
        let cached = self
            .sources
            .get(&key)
            .filter(|cached| identity.same(&cached.status))
            .cloned();
        let fresh = if let Some(cached) = cached {
            cached
        } else {
            let bytes = super::meta_bytes(&path, &mut budget);
            let Some(text) = bytes.ok().and_then(|bytes| String::from_utf8(bytes).ok()) else {
                round.incomplete.insert(group);
                return;
            };
            self.meta_record(identity, &text)
        };
        let session = fresh.session.clone();
        round.fresh.insert(key, fresh);
        round.tasks.push_back(Task::Journal {
            dir: dir.to_owned(),
            group,
            session,
            scan: None,
        });
    }

    /// What one meta records: its live seats and their priors.
    fn meta_record(&self, identity: Identity, text: &str) -> SourceRecord {
        let meta = crate::meta::Meta::parse(text);
        let session = meta.session_id().map(str::to_owned);
        let session_dir = meta.work_dir().map(PathBuf::from);
        let mut fresh = record(
            identity.state(identity.len, false, Vec::new()),
            session.clone(),
        );
        if session
            .as_ref()
            .is_some_and(|session| session.len() > FIELD_MAX)
        {
            fresh.session = None;
        }
        let home = self.home.as_deref();
        for entry in meta.roster() {
            let roots = roots_of(entry, session_dir.as_deref());
            let membership = |role: Role, client: RecordedClient| Membership {
                session: session.clone(),
                slot: entry.slot.clone(),
                role,
                client,
            };
            if let Some(id) = super::valid_id(entry.harness_session.as_deref()) {
                add(
                    &mut fresh,
                    home,
                    entry,
                    &id,
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
                    add(
                        &mut fresh,
                        home,
                        &seat,
                        &id,
                        membership(Role::Prior, RecordedClient::Missing),
                        &roots,
                    );
                }
            }
        }
        fresh
    }

    /// One unit of a session journal: its status, a witness chunk, or a
    /// chunk of new records.
    fn journal(
        &mut self,
        round: &mut Round,
        dir: PathBuf,
        group: String,
        session: Option<String>,
        scan: Option<Box<Scan>>,
    ) {
        let path = dir.join(crate::store::EVENTS);
        let next = match scan {
            None => self.journal_open(round, &path, &group, session.as_ref()),
            Some(scan) if scan.check.is_some() => {
                journal_check(round, &path, &group, session.as_ref(), scan)
            }
            Some(scan) => self.journal_chunk(round, &path, &group, session.as_ref(), scan),
        };
        if let Some(scan) = next {
            round.tasks.push_back(Task::Journal {
                dir,
                group,
                session,
                scan: Some(scan),
            });
        }
    }

    /// A journal's status: an unchanged one is taken from the cache, a grown
    /// one resumes at its cursor once its last block is proven, anything
    /// else is read from 0.
    fn journal_open(
        &self,
        round: &mut Round,
        path: &Path,
        group: &str,
        session: Option<&String>,
    ) -> Option<Box<Scan>> {
        // A missing journal is unavailable evidence, never an empty one: it
        // could have held a retire or reseat.
        let key = format!("journal:{group}");
        let Ok(Some(file)) = super::locate_transcript(path, &mut Budget::local(u64::MAX)) else {
            round.incomplete.insert(group.to_owned());
            return None;
        };
        let identity = Identity::of(&file.metadata);
        let cached = self
            .sources
            .get(&key)
            .filter(|cached| cached.session.as_ref() == session)
            .cloned();
        let scan = match cached {
            Some(cached) if identity.same(&cached.status) => {
                round.fresh.insert(key, cached);
                return None;
            }
            Some(cached) if !identity.replaced(&cached.status) && cached.status.cursor > 0 => {
                let all = identity.len == cached.status.len;
                match Check::of(&cached.status, all) {
                    Some(check) => Scan {
                        key,
                        lines: Lines::resume(
                            LINE_CAP,
                            cached.status.cursor,
                            cached.status.skipping,
                        ),
                        witness: Witness::new(),
                        check: Some(check),
                        damaged: !cached.complete,
                        record: cached,
                        anchor: None,
                    },
                    None => fresh_scan(key, identity, session.cloned()),
                }
            }
            _ => fresh_scan(key, identity, session.cloned()),
        };
        Some(Box::new(scan))
    }

    /// One chunk of new journal records from the cursor.
    fn journal_chunk(
        &self,
        round: &mut Round,
        path: &Path,
        group: &str,
        session: Option<&String>,
        mut scan: Box<Scan>,
    ) -> Option<Box<Scan>> {
        let Ok(window) = super::journal_window(path, scan.lines.cursor(), STEP) else {
            round.incomplete.insert(group.to_owned());
            return None;
        };
        if !scan.anchored(&window) {
            // Changed between chunks: what was read so far keeps the status
            // it was read under, never the new one, so the next round checks
            // the prefix again; this round lacks the journal.
            round.incomplete.insert(group.to_owned());
            let Scan {
                key,
                lines,
                witness,
                mut record,
                damaged,
                anchor,
                ..
            } = *scan;
            let anchor = anchor?;
            record.complete = !damaged;
            record.session = session.cloned();
            record.status = anchor.state(lines.cursor(), lines.skipping(), witness.blocks());
            round.fresh.insert(key, record);
            return None;
        }
        let mut damaged = false;
        let mut records = Vec::new();
        let consumed = scan.lines.step(&window.bytes, &mut |framed| match framed {
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
        let used = usize::try_from(consumed).unwrap_or(window.bytes.len());
        scan.witness
            .feed(&window.bytes[..used.min(window.bytes.len())]);
        for event in &records {
            journal_record(
                &mut scan.record,
                self.home.as_deref(),
                event,
                session.map(String::as_str),
            );
        }
        scan.damaged |= damaged;
        let identity = Identity::window(&window);
        if consumed > 0 && scan.lines.cursor() < window.len {
            return Some(scan);
        }
        let Scan {
            key,
            lines,
            witness,
            mut record,
            damaged,
            ..
        } = *scan;
        record.complete = !damaged;
        record.session = session.cloned();
        record.status = identity.state(lines.cursor(), lines.skipping(), witness.blocks());
        round.fresh.insert(key, record);
        None
    }

    fn archives(&mut self, round: &mut Round) {
        let root = self.state_root.clone();
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
                        round.tasks.push_back(Task::Archive {
                            dir: dir.join(&id),
                            id,
                        });
                    }
                }
            }
            Ok(Bounded::Truncated) | Err(_) => round.archives_listed = false,
        }
    }

    fn archive(&mut self, round: &mut Round, dir: &Path, id: &str) {
        let key = format!("archive:{id}");
        let path = dir.join(crate::archive::harness::FILE);
        let mut budget = Budget::local(u64::MAX);
        // A kept archive without its harness record may hide memberships.
        let Ok(Some(file)) = super::locate_transcript(&path, &mut budget) else {
            round.incomplete.insert(key);
            return;
        };
        let identity = Identity::of(&file.metadata);
        if let Some(cached) = self
            .sources
            .get(&key)
            .filter(|cached| identity.same(&cached.status))
        {
            round.fresh.insert(key, cached.clone());
            return;
        }
        let Ok(bytes) = super::bounded_bytes(&path, HARNESS_MAX, &mut budget) else {
            round.incomplete.insert(key);
            return;
        };
        let Ok(harness) = crate::archive::harness::parse(&bytes, id) else {
            round.incomplete.insert(key);
            round.archives_listed = false;
            return;
        };
        let mut fresh = record(identity.state(identity.len, false, Vec::new()), None);
        let home = self.home.clone();
        for slot in &harness.slots {
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
                    session: Some(harness.archive_id.clone()),
                    slot: slot.slot.clone(),
                    role: Role::Archived,
                    client: RecordedClient::Missing,
                };
                add(
                    &mut fresh,
                    home.as_deref(),
                    &seat,
                    &conversation.id,
                    membership,
                    &roots,
                );
            }
        }
        round.fresh.insert(key, fresh);
    }

    /// Association is done: derive every conversation's memberships, groups,
    /// stores and roots from the sources in key order, so a warm round and
    /// a cold one derive the same.
    fn associate(&mut self, round: &mut Round) {
        for (key, record) in &round.fresh {
            let group = group_of(key);
            if !whole(key, record) {
                round.incomplete.insert(group.clone());
            }
            round.oversize = round.oversize.saturating_add(record.oversize);
            round.reseated.extend(record.reseats.iter().cloned());
            for (tool, id, session) in &record.unsupported {
                round
                    .unsupported
                    .entry((tool.clone(), id.clone()))
                    .or_default()
                    .insert(session.clone());
            }
            for seat in &record.seats {
                let assoc = round
                    .assoc
                    .entry((seat.adapter, seat.id.clone()))
                    .or_default();
                if !assoc.memberships.contains(&seat.membership) {
                    assoc.memberships.push(seat.membership.clone());
                }
                assoc.groups.insert(group.clone());
                if let Some(store) = &seat.store {
                    assoc.stores.insert(PathBuf::from(store));
                }
                for home in &seat.config_homes {
                    let home = PathBuf::from(home);
                    if !assoc.roots.config_homes.contains(&home) {
                        assoc.roots.config_homes.push(home);
                    }
                }
                for dir in &seat.work_dirs {
                    let dir = PathBuf::from(dir);
                    if !assoc.roots.work_dirs.contains(&dir) {
                        assoc.roots.work_dirs.push(dir);
                    }
                }
                assoc.roots.config_homes.sort();
                assoc.roots.work_dirs.sort();
            }
        }
        if round.fresh != self.sources {
            self.sources.clone_from(&round.fresh);
            self.dirty.sources = true;
        }
        // Any source whose evidence is unavailable may hide a membership of
        // ANY conversation, so no client is proven this round; the count is
        // a path-free coverage line.
        self.hidden = round.incomplete.len()
            + usize::from(!round.archives_listed)
            + usize::from(!round.sessions_listed);
        self.oversize = round.oversize;
        self.unsupported = std::mem::take(&mut round.unsupported);
        let hidden = self.hidden > 0 || self.oversize > 0;
        let mut association = Memberships::new();
        for (key, assoc) in &round.assoc {
            let complete = !hidden
                && assoc
                    .groups
                    .iter()
                    .all(|group| !round.incomplete.contains(group));
            association.insert(
                key.clone(),
                Association {
                    memberships: assoc.memberships.clone(),
                    evidence_complete: complete,
                    reseated: round.reseated.contains(&key.1),
                },
            );
        }
        if association != self.association {
            self.association = association;
            self.dirty.sources = true;
        }
    }
}

/// A journal scan from 0.
fn fresh_scan(key: String, identity: Identity, session: Option<String>) -> Scan {
    Scan {
        key,
        lines: Lines::new(LINE_CAP),
        witness: Witness::new(),
        check: None,
        record: record(identity.state(0, false, Vec::new()), session),
        damaged: false,
        anchor: None,
    }
}

/// One chunk re-hashing a journal's consumed prefix: a mismatch reads the
/// journal again from 0.
fn journal_check(
    round: &mut Round,
    path: &Path,
    group: &str,
    session: Option<&String>,
    mut scan: Box<Scan>,
) -> Option<Box<Scan>> {
    let mut check = scan.check.take()?;
    let take = STEP.min(check.end.saturating_sub(check.at));
    let Ok(window) = super::journal_window(path, check.at, take) else {
        round.incomplete.insert(group.to_owned());
        return None;
    };
    if !scan.anchored(&window) {
        // Changed during the check: the cached record stands under its own
        // status, so the next round checks again; this round lacks it.
        round.incomplete.insert(group.to_owned());
        let Scan { key, record, .. } = *scan;
        round.fresh.insert(key, record);
        return None;
    }
    let same = check.feed(&window.bytes) && (check.at < check.end || check.closes());
    if !same || window.bytes.is_empty() && check.at < check.end {
        // Not the prefix it was: read again from 0.
        let key = scan.key.clone();
        return Some(Box::new(fresh_scan(
            key,
            Identity::window(&window),
            session.cloned(),
        )));
    }
    if check.at < check.end {
        scan.check = Some(check);
    } else {
        scan.witness = check.witness;
    }
    Some(scan)
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
        round.tasks.push_back(Task::ClaudeStore { store });
    }
    let mut days = BTreeSet::new();
    for (root, id) in &codex {
        if let Some(dirs) = crate::quota::codex_rollout_dirs(root, id) {
            days.extend(dirs);
        } else {
            // No id day: proven absent.
            round.rollouts.insert((root.clone(), id.clone()), Ok(None));
        }
    }
    for dir in days {
        round.tasks.push_back(Task::CodexDay { dir });
    }
    for (root, id) in codex {
        if !round.rollouts.contains_key(&(root.clone(), id.clone())) {
            round.tasks.push_back(Task::CodexFind { root, id });
        }
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

/// An unsupported seat's tool as the coverage line may keep it: the
/// recorded binary's base name only when it passes the plan §3 identity
/// rule, else the fixed word `unresolved`. Normalized BEFORE it is retained.
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

// --- Locating (plan §6) ----------------------------------------------------

fn claude_store(round: &mut Round, store: PathBuf) {
    let mut budget = Budget::local(u64::MAX);
    let projects = match super::derived_directory(&store, "projects", &mut budget) {
        Ok(Some(projects)) => projects,
        Ok(None) => return,
        Err(_) => {
            round.broken.insert(store);
            return;
        }
    };
    match super::names_only(&projects, &mut budget) {
        Ok(Bounded::Ready(names)) => {
            for (name, kind) in names {
                if kind == Entry::Dir {
                    round.tasks.push_back(Task::ClaudeProject {
                        store: store.clone(),
                        project: projects.join(name),
                    });
                }
            }
        }
        Ok(Bounded::Truncated) | Err(_) => {
            round.broken.insert(store);
        }
    }
}

fn claude_project(round: &mut Round, store: PathBuf, project: &Path) {
    let names = match super::names_only(project, &mut Budget::local(u64::MAX)) {
        Ok(Bounded::Ready(names)) => names,
        Ok(Bounded::Truncated) | Err(_) => {
            round.broken.insert(store);
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
        round
            .parents
            .entry((store.clone(), id))
            .or_default()
            .push(path);
    }
    for (id, dir) in children {
        round.tasks.push_back(Task::Subagents {
            store: store.clone(),
            id,
            dir,
        });
    }
}

fn subagents(round: &mut Round, store: PathBuf, id: String, dir: &Path) {
    let mut budget = Budget::local(u64::MAX);
    let subagents = match super::derived_directory(dir, "subagents", &mut budget) {
        Ok(Some(subagents)) => subagents,
        Ok(None) => return,
        Err(_) => {
            round.broken.insert(store);
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
                    kept.insert(stem.to_owned(), subagents.join(&name));
                }
            }
            round.children.entry((store, id)).or_default().extend(kept);
        }
        Ok(Bounded::Truncated) | Err(_) => {
            round.broken.insert(store);
        }
    }
}

fn codex_day(round: &mut Round, dir: &Path) {
    // The cache keeps a complete listing (or a proven absence); a failed or
    // cut one is not cached, and the find then reports its ids unreadable
    // without listing it again.
    if let Ok(Bounded::Ready(_)) = round.days.names(dir, &mut Budget::local(u64::MAX)) {
        round.listed_days.insert(dir.to_owned());
    }
}

fn codex_find(round: &mut Round, root: PathBuf, id: String) {
    // The find only consults day listings its own units cached: a day whose
    // listing failed or was cut is never listed again inside this unit.
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
    round.rollouts.insert((root, id), outcome);
}

/// What locating found for one conversation.
struct Found {
    files: Vec<(Place, PathBuf)>,
    parent: bool,
    gaps: Vec<GapKind>,
}

/// The files the round's listings located for conversation `key`, whether
/// one is its parent, and the gaps its stores show.
fn found(round: &Round, key: &Key, assoc: &Assoc) -> Found {
    let mut found = Found {
        files: Vec::new(),
        parent: false,
        gaps: Vec::new(),
    };
    for store in &assoc.stores {
        if round.broken.contains(store) {
            found.gaps.push(GapKind::Unreadable);
            continue;
        }
        let copy = copy_of(store);
        let at = |role: StreamRole| Place {
            conversation: key.clone(),
            stream: StreamKey { role, copy },
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
                for (stem, path) in round.children.get(&pair).into_iter().flatten() {
                    found
                        .files
                        .push((at(StreamRole::Child(child_id(stem))), path.clone()));
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

impl Reader {
    /// Locating is done: every associated conversation's files and gaps;
    /// its held state loaded first, then the lstats.
    fn place(&mut self, round: &mut Round) {
        let keys: Vec<Key> = round.assoc.keys().cloned().collect();
        let mut conversations = Vec::with_capacity(keys.len());
        for key in &keys {
            let Some(assoc) = round.assoc.get(key) else {
                continue;
            };
            let found = found(round, key, assoc);
            let mut marks = found.gaps;
            if assoc
                .groups
                .iter()
                .any(|group| round.incomplete.contains(group))
            {
                marks.push(GapKind::Unreadable);
            }
            if !found.parent && !marks.contains(&GapKind::NotUnique) {
                marks.push(GapKind::Unlocated);
            }
            marks.sort();
            marks.dedup();
            if found.parent {
                round.with_parent.insert(key.clone());
            }
            let association = self.association.get(key);
            conversations.push(Conversation {
                adapter: key.0,
                id: key.1.clone(),
                memberships: assoc.memberships.clone(),
                evidence_complete: association.is_some_and(|one| one.evidence_complete),
                reseated: association.is_some_and(|one| one.reseated),
                streams: Vec::new(),
                gaps: marks.clone(),
                read: false,
            });
            round.marks.insert(key.clone(), marks);
            let open: BTreeSet<StreamKey> = found
                .files
                .iter()
                .map(|(place, _)| place.stream.clone())
                .collect();
            round.open.insert(key.clone(), open);
            round.located.extend(found.files);
        }
        self.conversations = conversations;
        // The view lists the associated conversations only.
        let gone: Vec<Key> = self
            .all
            .convs
            .iter()
            .map(|one| (one.summary.adapter, one.summary.id.clone()))
            .filter(|key| !round.assoc.contains_key(key))
            .collect();
        for key in gone {
            let _ = self.enter(&key, None);
        }
        for key in &keys {
            if self.convs.contains_key(key) {
                continue;
            }
            if self.durable && self.proof != Proof::Cold && self.epoch.is_some() {
                self.hold(round, key);
            } else {
                self.convs.insert(
                    key.clone(),
                    Held {
                        conv: Conv::new(key.0, &key.1),
                        dirty: true,
                        partial: false,
                    },
                );
            }
        }
        let batch: Vec<(Place, PathBuf)> = round
            .located
            .iter()
            .map(|(place, path)| (place.clone(), path.clone()))
            .collect();
        for chunk in batch.chunks(usize::try_from(LSTATS).unwrap_or(256)) {
            round.tasks.push_back(Task::Lstats(chunk.to_vec()));
        }
        for key in &keys {
            if self.convs.contains_key(key) && round.open.get(key).is_some_and(BTreeSet::is_empty) {
                self.settle(round, key);
            }
        }
    }

    /// Hold `key` from its view entry when the view has its head; its
    /// document is loaded only before a change (plan r4 section 1). Without
    /// a head, the document is loaded now. A head without file states (an
    /// unlocated conversation) is still a head.
    fn hold(&mut self, round: &mut Round, key: &Key) {
        let entry = self
            .all
            .convs
            .binary_search_by(|one| {
                (one.summary.adapter, one.summary.id.as_str()).cmp(&(key.0, key.1.as_str()))
            })
            .ok()
            .and_then(|index| self.all.convs.get(index))
            .and_then(|entry| Some((&entry.summary, entry.head.as_ref()?)));
        let Some((summary, head)) = entry else {
            round.tasks.push_back(Task::LoadConv(key.clone()));
            return;
        };
        let conv = Conv::partial(summary.clone(), head);
        self.convs.insert(
            key.clone(),
            Held {
                conv,
                dirty: false,
                partial: true,
            },
        );
    }
}

// --- Reading (plan r4 §3) ----------------------------------------------------

impl Reader {
    fn lstats(&mut self, round: &mut Round, batch: Vec<(Place, PathBuf)>, guard: &Guard) {
        let mut batch = batch.into_iter();
        while let Some((place, path)) = batch.next() {
            // One lstat per located file: the rest waits for its own unit once
            // this one's receipt reaches the bound.
            if stats(guard) >= u64::from(LSTATS) {
                let rest: Vec<(Place, PathBuf)> =
                    std::iter::once((place, path)).chain(batch).collect();
                round.tasks.push_back(Task::Lstats(rest));
                break;
            }
            if round.work.contains_key(&place) {
                // Queued already by a rebuild of its conversation.
                continue;
            }
            let key = place.conversation.clone();
            let Some(held) = self.convs.get(&key) else {
                // Its document could not be read: not read this round.
                self.done(round, &place);
                continue;
            };
            if held.conv.summary.refused.is_some() {
                // Refused whole: only a reset retries it.
                self.done(round, &place);
                continue;
            }
            // A conversation held from its view entry is loaded before its
            // roots change it; this file is checked again after the load.
            if held.partial && held.conv.resume.roots() != classified_by(round, &key).as_ref() {
                let rest: Vec<(Place, PathBuf)> =
                    std::iter::once((place, path)).chain(batch).collect();
                load_first(round, key, rest);
                break;
            }
            // Its events were classified by other roots: it is rebuilt, and
            // this file is still checked by its own lstat below.
            self.reclassify(round, &key);
            let Some(held) = self.convs.get(&key) else {
                continue;
            };
            let file = match super::locate_transcript(&path, &mut Budget::local(u64::MAX)) {
                Ok(Some(file)) => {
                    round.statted.insert(place.clone());
                    file
                }
                Ok(None) => {
                    round.missing.insert(place.clone());
                    self.done(round, &place);
                    continue;
                }
                Err(_) => {
                    round.changed.insert(place.clone());
                    self.done(round, &place);
                    continue;
                }
            };
            let identity = Identity::of(&file.metadata);
            let state = held.conv.resume.file(&place.stream).cloned();
            if held.partial && !state.as_ref().is_some_and(|state| identity.same(state)) {
                // Its status moved: the document is loaded, then this file
                // is checked again against the full resume state.
                let rest: Vec<(Place, PathBuf)> =
                    std::iter::once((place, path)).chain(batch).collect();
                load_first(round, key, rest);
                break;
            }
            match state {
                None => self.start(round, place, path, identity),
                Some(state) if identity.same(&state) => self.done(round, &place),
                Some(state) if identity.replaced(&state) => {
                    self.rebuild(round, &key);
                }
                Some(state) if state.cursor == 0 => self.start(round, place, path, identity),
                Some(state) => {
                    let all = identity.len == state.len;
                    match Check::of(&state, all) {
                        Some(check) => {
                            self.next += 1;
                            round.work.insert(
                                place.clone(),
                                Work {
                                    generation: self.next,
                                    stage: Stage::Verifying(Box::new(Verifying {
                                        path,
                                        anchor: identity,
                                        check,
                                        state,
                                    })),
                                },
                            );
                            round.tasks.push_back(Task::Verify {
                                place,
                                generation: self.next,
                            });
                        }
                        None => self.rebuild(round, &key),
                    }
                }
            }
        }
    }

    /// Read `place` from 0.
    fn start(&mut self, round: &mut Round, place: Place, path: PathBuf, anchor: Identity) {
        let roots = round
            .assoc
            .get(&place.conversation)
            .map(|assoc| assoc.roots.clone())
            .unwrap_or_default();
        self.next += 1;
        round.work.insert(
            place.clone(),
            Work {
                generation: self.next,
                stage: Stage::Reading(Box::new(Reading {
                    path,
                    roots,
                    lines: Lines::new(LINE_CAP),
                    witness: Witness::new(),
                    anchor,
                    identity: anchor,
                    events: Vec::new(),
                    malformed: 0,
                    over_cap: 0,
                    took: false,
                    done: false,
                    fresh: true,
                })),
            },
        );
        round.tasks.push_back(Task::Read {
            place,
            generation: self.next,
        });
    }

    /// Rebuild `key` when its held events were classified by other recorded
    /// roots than this round's. A conversation with no file read yet only
    /// takes this round's roots. A client label is not a root, so a relabel
    /// reads nothing.
    fn reclassify(&mut self, round: &mut Round, key: &Key) {
        let want = classified_by(round, key);
        let Some(held) = self.convs.get_mut(key) else {
            return;
        };
        if held.conv.resume.roots() == want.as_ref() {
            return;
        }
        if held.conv.summary.files.is_empty() {
            held.conv.resume.set_roots(want);
            return;
        }
        self.rebuild(round, key);
    }

    /// The one rebuild seam (P6 holds across every trigger): `key` is read
    /// again from its sources under this round's roots, and no evidence is
    /// dropped here ([`Conv::rebuild`]). A file whose own lstat already
    /// found it on disk this round is read again from 0 now. Every other
    /// located file is read from 0 by its own lstat when that finds it, and
    /// is classified gone, with its counts kept, when it does not. A read
    /// from 0 replaces its stream's evidence only when it is taken.
    fn rebuild(&mut self, round: &mut Round, key: &Key) {
        let roots = classified_by(round, key);
        let places: Vec<(Place, PathBuf)> = round
            .located
            .iter()
            .filter(|(place, _)| place.conversation == *key && !round.missing.contains(place))
            .map(|(place, path)| (place.clone(), path.clone()))
            .collect();
        let Some(held) = self.convs.get_mut(key) else {
            return;
        };
        held.conv.rebuild();
        held.conv.resume.set_roots(roots);
        held.dirty = true;
        round.rebuilt.insert(key.clone());
        self.recounting = self
            .recounting
            .saturating_add(u64::try_from(places.len()).unwrap_or(u64::MAX));
        let statted: Vec<(Place, PathBuf)> = places
            .into_iter()
            .filter(|(place, _)| round.statted.contains(place))
            .collect();
        let open = round.open.entry(key.clone()).or_default();
        for (place, _) in &statted {
            open.insert(place.stream.clone());
        }
        for (place, path) in statted {
            round.work.remove(&place);
            round.changed.remove(&place);
            let anchor = Identity::default();
            self.start(round, place.clone(), path, anchor);
            // A read from 0 takes the status its first chunk sees.
            if let Some(Work {
                stage: Stage::Reading(reading),
                ..
            }) = round.work.get_mut(&place)
            {
                reading.took = false;
                reading.anchor = Identity::default();
            }
        }
    }

    /// One chunk re-hashing the consumed prefix. Every chunk must see the
    /// status the pass started from; a file that changed under the pass ends
    /// it as changed, and the next round checks again.
    fn verify(&mut self, round: &mut Round, place: Place, generation: u64) {
        let Some(work) = round.work.get_mut(&place) else {
            return;
        };
        if work.generation != generation {
            return;
        }
        let Stage::Verifying(verifying) = &mut work.stage else {
            return;
        };
        let Verifying {
            path,
            anchor,
            check,
            state,
        } = &mut **verifying;
        let take = STEP.min(check.end.saturating_sub(check.at));
        let window = super::journal_window(path, check.at, take);
        let Ok(window) = window else {
            round.changed.insert(place.clone());
            round.work.remove(&place);
            self.done(round, &place);
            return;
        };
        if Identity::window(&window) != *anchor {
            round.changed.insert(place.clone());
            round.work.remove(&place);
            self.done(round, &place);
            return;
        }
        let fed = check.feed(&window.bytes);
        if !fed || window.bytes.is_empty() && check.at < check.end {
            let key = place.conversation;
            self.rebuild(round, &key);
            return;
        }
        if check.at < check.end {
            round.tasks.push_back(Task::Verify { place, generation });
            return;
        }
        if !check.closes() {
            let key = place.conversation;
            self.rebuild(round, &key);
            return;
        }
        // The read resumes under the status the prefix was verified at.
        let witness = check.witness.clone();
        let lines = Lines::resume(LINE_CAP, state.cursor, state.skipping);
        let path = path.clone();
        let anchor = *anchor;
        let roots = round
            .assoc
            .get(&place.conversation)
            .map(|assoc| assoc.roots.clone())
            .unwrap_or_default();
        work.stage = Stage::Reading(Box::new(Reading {
            path,
            roots,
            lines,
            witness,
            anchor,
            identity: anchor,
            events: Vec::new(),
            malformed: 0,
            over_cap: 0,
            took: false,
            done: false,
            fresh: false,
        }));
        round.tasks.push_back(Task::Read { place, generation });
    }

    /// One chunk of new bytes from the cursor. Each chunk must see the
    /// status the previous one saw; otherwise the file changed between
    /// units: the chunk is dropped and the file ends this round as changed
    /// with its last consistent state, so the next round checks again.
    fn read(&mut self, round: &mut Round, place: Place, generation: u64) {
        let Some(work) = round.work.get_mut(&place) else {
            return;
        };
        if work.generation != generation {
            return;
        }
        let Stage::Reading(reading) = &mut work.stage else {
            return;
        };
        let window = super::journal_window(&reading.path, reading.lines.cursor(), STEP);
        let Ok(window) = window else {
            round.changed.insert(place.clone());
            self.done(round, &place);
            return;
        };
        let identity = Identity::window(&window);
        let first = reading.anchor == Identity::default();
        if !first && identity != reading.anchor {
            round.changed.insert(place.clone());
            self.done(round, &place);
            return;
        }
        let adapter = place.conversation.0;
        let mut parsed = Vec::new();
        let mut malformed = 0_u64;
        let mut over_cap = 0_u64;
        let consumed = {
            let claude = claude_calls::Roots {
                config_homes: &reading.roots.config_homes,
                work_dirs: &reading.roots.work_dirs,
            };
            let events = &mut reading.events;
            reading
                .lines
                .step(&window.bytes, &mut |framed| match framed {
                    Framed::OverCap => over_cap += 1,
                    Framed::Line(line) => {
                        let outcome = match adapter {
                            Adapter::Claude => claude_calls::line(line, &claude, &mut parsed),
                            Adapter::Codex => codex_calls::line(line, &mut parsed),
                        };
                        if outcome == Parsed::Malformed {
                            malformed += 1;
                        }
                        events.append(&mut parsed);
                    }
                })
        };
        let used = usize::try_from(consumed).unwrap_or(window.bytes.len());
        reading
            .witness
            .feed(&window.bytes[..used.min(window.bytes.len())]);
        reading.malformed = reading.malformed.saturating_add(malformed);
        reading.over_cap = reading.over_cap.saturating_add(over_cap);
        reading.anchor = identity;
        reading.identity = identity;
        reading.took = true;
        if consumed > 0 && reading.lines.cursor() < identity.len {
            round.tasks.push_back(Task::Read { place, generation });
            return;
        }
        reading.done = true;
        self.done(round, &place);
    }

    /// `place` is settled this round.
    fn done(&mut self, round: &mut Round, place: &Place) {
        let key = place.conversation.clone();
        let Some(open) = round.open.get_mut(&key) else {
            return;
        };
        open.remove(&place.stream);
        if open.is_empty() && self.convs.contains_key(&key) {
            self.settle(round, &key);
        }
    }

    /// Every located file of `key` is settled: take what was read into its
    /// conversation, mark its sources, gaps and read state, and publish.
    fn settle(&mut self, round: &mut Round, key: &Key) {
        let places: Vec<Place> = round
            .work
            .keys()
            .filter(|place| place.conversation == *key)
            .cloned()
            .collect();
        let (marks, read) = settled_marks(round, key);
        let Some(held) = self.convs.get_mut(key) else {
            return;
        };
        if held.partial
            && (!places.is_empty()
                || round.rebuilt.contains(key)
                || goes(round, key, &held.conv)
                || held.conv.resume.marks() != marks.as_slice()
                || held.conv.summary.read != read)
        {
            // Held from its view entry and about to change: its document is
            // loaded first, and the load settles it again.
            round.tasks.push_front(Task::LoadConv(key.clone()));
            return;
        }
        let before = held.conv.summary.clone();
        let (moved, overflow) = take_readings(round, places, &mut held.conv, &self.caps);
        if overflow {
            // Counts no source makes: rebuilt from the sources next round.
            held.conv.rebuild();
            held.dirty = true;
        }
        mark_gone(round, key, &mut held.conv);
        if held.conv.set_gaps(marks.clone()).is_err() {
            held.dirty = true;
        }
        held.conv.summary.read = read;
        if moved || held.conv.summary != before || round.rebuilt.contains(key) {
            let caps = self.caps;
            held.conv.bound(&caps, |conv| {
                u64::try_from(doc::render_conv(conv).len()).unwrap_or(u64::MAX)
            });
            held.dirty = true;
        }
        // The records are built in key order: one search, not a scan.
        if let Ok(index) = self
            .conversations
            .binary_search_by(|one| (one.adapter, one.id.as_str()).cmp(&(key.0, key.1.as_str())))
        {
            self.conversations[index].read = read;
            self.conversations[index].gaps = marks;
        }
        let mut published = incr::publish(&held.conv.summary, self.association.get(key));
        published.head = Some(held.conv.head());
        let mut dirty = held.dirty;
        if self.enter(key, Some(published)).is_err()
            && let Some(held) = self.convs.get_mut(key)
        {
            // The view could not add it up: rebuilt from its sources.
            held.conv.rebuild();
            held.dirty = true;
            dirty = true;
        }
        if dirty && self.publishing() {
            round.tasks.push_back(Task::Publish(key.clone()));
        } else if self.publishing()
            && let Some(held) = self.convs.get_mut(key)
        {
            // Settled with nothing to publish: only its view entry is kept.
            release(held);
        }
    }
}

/// The digest of the roots this round classifies `key`'s events by: Claude
/// only, as Codex events name no skill directory.
fn classified_by(round: &Round, key: &Key) -> Option<crate::tool_stats::Key> {
    match key.0 {
        Adapter::Claude => round.assoc.get(key).map(|assoc| roots_digest(&assoc.roots)),
        Adapter::Codex => None,
    }
}

/// Take what was read of `places` into `conv`: each file's events, line
/// counts and new state. Returns whether a file state moved and whether a
/// count overflowed (the conversation is then rebuilt).
fn take_readings(
    round: &mut Round,
    places: Vec<Place>,
    conv: &mut Conv,
    caps: &Caps,
) -> (bool, bool) {
    let mut moved = false;
    let mut taken: Vec<(Place, Reading)> = Vec::new();
    for place in places {
        let Some(work) = round.work.remove(&place) else {
            continue;
        };
        let Stage::Reading(reading) = work.stage else {
            continue;
        };
        if reading.took {
            taken.push((place, *reading));
        }
    }
    // Committed only now, and all before any admission: every taken read
    // from 0 drops what its stream held, so the cap sees the conversation
    // as it will be, never new evidence beside the superseded one (R4-1).
    // Counts only grow as evidence is added, so the final conversation
    // fitting means every step toward it fits. A read from 0 that took
    // nothing, and a gone stream, keep their evidence as it was.
    for (place, reading) in &taken {
        if reading.fresh {
            conv.restart(&place.stream);
        }
    }
    for (place, reading) in taken {
        let stream = &place.stream;
        conv.apply(stream, &reading.events, caps);
        if conv
            .note(stream, reading.malformed, reading.over_cap)
            .is_err()
        {
            return (moved, true);
        }
        let state = reading.identity.state(
            reading.lines.cursor(),
            reading.lines.skipping(),
            reading.witness.blocks(),
        );
        if conv.resume.file(stream) != Some(&state) {
            conv.resume.set_file(stream.clone(), state);
            moved = true;
        }
        conv.source_present(stream);
    }
    (moved, false)
}

/// A counted file no longer listed, or gone at its lstat, keeps its counts,
/// visibly (P6); a store whose listing failed proves nothing.
/// This round's discovery marks of `key` and whether it is read: what a
/// settle sets.
fn settled_marks(round: &Round, key: &Key) -> (Vec<GapKind>, bool) {
    let mut marks = round.marks.get(key).cloned().unwrap_or_default();
    let changed = round.changed.iter().any(|place| place.conversation == *key);
    if changed && !marks.contains(&GapKind::Changed) {
        marks.push(GapKind::Changed);
        marks.sort();
    }
    let read = round.with_parent.contains(key)
        && !changed
        && !marks.contains(&GapKind::Unlocated)
        && !marks.contains(&GapKind::Changed);
    (marks, read)
}

/// Whether [`mark_gone`] would mark a source of `conv` gone.
fn goes(round: &Round, key: &Key, conv: &Conv) -> bool {
    let broken: BTreeSet<[u8; 16]> = round.broken.iter().map(|store| copy_of(store)).collect();
    conv.summary.files.iter().any(|(stream, source)| {
        let place = Place {
            conversation: key.clone(),
            stream: stream.clone(),
        };
        let listed = round.located.contains_key(&place) && !round.missing.contains(&place);
        *source == Source::Present && !listed && !broken.contains(&stream.copy)
    })
}

fn mark_gone(round: &Round, key: &Key, conv: &mut Conv) {
    let broken: BTreeSet<[u8; 16]> = round.broken.iter().map(|store| copy_of(store)).collect();
    let streams: Vec<StreamKey> = conv
        .summary
        .files
        .iter()
        .filter(|(_, source)| **source == Source::Present)
        .map(|(stream, _)| stream.clone())
        .collect();
    for stream in streams {
        let place = Place {
            conversation: key.clone(),
            stream: stream.clone(),
        };
        let listed = round.located.contains_key(&place) && !round.missing.contains(&place);
        if !listed && !broken.contains(&stream.copy) {
            conv.source_gone(&stream);
        }
    }
}

/// Keep only what the view holds of a settled, durable conversation: its
/// summary and head (plan r4 section 1: a Resume is held only while its
/// conversation is processed).
fn release(held: &mut Held) {
    if !held.partial {
        let kept = held.conv.head();
        held.conv = Conv::partial(held.conv.summary.clone(), &kept);
        held.partial = true;
    }
}

/// Load `key`'s document before `rest` is checked again: what a
/// conversation held from its view entry needs before anything changes it.
fn load_first(round: &mut Round, key: Key, rest: Vec<(Place, PathBuf)>) {
    round.tasks.push_front(Task::Lstats(rest));
    round.tasks.push_front(Task::LoadConv(key));
}

/// A cold reader run through one whole round: it never reads or changes
/// `<root>/stats`.
#[cfg(test)]
pub(crate) fn scan(state_root: PathBuf, home: Option<PathBuf>, caps: Caps) -> Reader {
    let mut reader = Reader::with(state_root, home, caps, false);
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
#[path = "calls_lazy_spec.rs"]
mod lazy_spec;

#[cfg(test)]
#[path = "calls_read_tests.rs"]
mod tests;
