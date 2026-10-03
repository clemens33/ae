//! The chat's "needs you" section: every roster seat whose attention verdict
//! says the human is needed, as a SNAPSHOT of the current set.
//!
//! PURE — no clock, no I/O, no tmux. The verdict is the one `ae list` prints,
//! `AgentEntry.reason`, computed by the caller through `session::entry_from`;
//! this module attributes it to its evidence and names every reason it cannot
//! be trusted.
//! What is printed when, and how, is the view's concern.

use crate::attention::Reason;
use crate::digest::SessionEntry;
use crate::session::{RecordSnapshot, SessionRuntime};
use crate::time::Timestamp;
use crate::tmux::Evidence;

/// One roster seat, by the two facts that identify it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeatRef {
    /// `main` / `worker.<n>` / `spawned.<n>`.
    pub slot: String,
    /// The seat's name.
    pub name: String,
}

/// Why a seat's verdict cannot be trusted. One per seat, in this precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    /// The watchdog has never beaten for this session.
    WatchdogOff,
    /// The watchdog's beat is there and could not be read.
    WatchdogUnreadable,
    /// The watchdog's last beat is older than [`STALE_SECS`].
    WatchdogStale {
        /// When it last beat, epoch micros.
        last_micros: i64,
    },
    /// tmux did not answer which panes the session has.
    RuntimeUnread,
    /// tmux answered, but which pane is this seat's could not be proven.
    PaneUnproven,
    /// The journal was read with lines it could not parse.
    JournalPartial {
        /// How many lines were skipped.
        skipped: usize,
    },
}

/// What a seat's row says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The seat's attention reason, as `ae list` reads it.
    Reason(Reason),
    /// No reason is known and the source that would say so cannot be trusted.
    Unknown(Cause),
}

/// The evidence a row's reason is attributed to. It decides nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A journal alert the watchdog booked, by its action.
    Alert {
        /// The record's action, e.g. `human-prompt`, `limit`, `alert`.
        action: String,
    },
    /// The seat's current declaration.
    Declaration,
    /// No pane carries the seat's slot now.
    NoPane,
    /// A reason no evidence above explains.
    Unattributed,
    /// The row is an [`Verdict::Unknown`].
    Unverified,
}

/// One seat that needs the human, or whose verdict cannot be trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub seat: SeatRef,
    /// The seat is one of the session's lead pair.
    pub lead_pair: bool,
    pub verdict: Verdict,
    pub source: Source,
    /// A reason row whose source is off, stale or unreadable.
    pub stale: Option<Cause>,
    /// The deciding record's stamp, epoch micros.
    pub since_micros: Option<i64>,
    /// The deciding record's summary, RAW: the view makes it inert.
    pub detail: String,
    /// The deciding record's position in the snapshot's events.
    pub record: Option<usize>,
}

/// The current set, most severe first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    pub rows: Vec<Row>,
}

/// A beat older than this — three watchdog cycles at the 60 s default — is stale.
pub const STALE_SECS: i64 = 180;

/// Everything the fold reads, each already read by its own owner.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// The session's name.
    pub session: &'a str,
    /// The session's meta and journal, read together.
    pub snapshot: &'a RecordSnapshot,
    /// The session's entry, computed from `snapshot` and `runtime` by
    /// `session::entry_from`: each agent's `reason` is the verdict.
    pub entry: &'a SessionEntry,
    /// What tmux said of the session's panes.
    pub runtime: &'a SessionRuntime,
    /// Whether tmux answered at all.
    pub runtime_read: bool,
    /// The watchdog's beat.
    pub beat: Evidence,
    /// The meta's `layout` is `lead-pair`, so `worker.0` is of the lead pair.
    pub lead_pair: bool,
    /// Now.
    pub now: Timestamp,
}

/// The current set.
///
/// # Errors
///
/// Why there is no settled read: the view keeps the rows it showed and says so.
pub fn fold(inputs: &Inputs<'_>) -> Result<Section, String> {
    let _ = inputs;
    Ok(Section::default())
}
