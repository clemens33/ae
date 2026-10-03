//! The sidebar's rows: one per session, in fleet order, two lines each.

use std::collections::BTreeMap;

use crate::console::needs::Section;
use crate::listing::World;
use crate::theme::{FleetOrder, Mark};
use crate::time::Timestamp;
use crate::tmux::PickerAgent;

/// What is known of a session's seats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Facts {
    /// The watchdog's published roster, with the session's tmux id.
    Seats {
        /// The tmux session id, `$n`.
        id: String,
        /// The seats, as the picker reads them.
        agents: Vec<PickerAgent>,
    },
    /// The session runs on another tmux server.
    OtherServer,
    /// The session publishes no roster fact.
    NotPublished,
}

/// The sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fleet {
    pub rows: Vec<Row>,
    /// The session this app was opened for.
    pub home: Option<String>,
}

/// One session's two lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    /// The sidebar's own 1-based index.
    pub index: usize,
    pub mark: Mark,
    pub needy: bool,
    pub counts: Counts,
    pub line2: Line2,
    pub home: bool,
}

/// The right side of line 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Counts {
    /// Mark, blocked, count: urgency order, non-zero only.
    Marks(Vec<(Mark, bool, usize)>),
    /// No seat facts: drawn `?`.
    Unknown,
    /// The session is stopped.
    Stopped,
}

/// Line 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line2 {
    Question {
        text: String,
        age_secs: Option<i64>,
    },
    SeatDown,
    Goal {
        text: String,
        age_secs: Option<i64>,
    },
    NoGoal,
    NotRunning {
        seats: usize,
        stopped_secs: Option<i64>,
    },
}

/// The sidebar's rows.
#[must_use]
pub fn rows(
    _world: &World,
    _facts: &BTreeMap<String, Facts>,
    _last_live: &BTreeMap<String, i64>,
    _order: &FleetOrder,
    _needs: &BTreeMap<String, Section>,
    home: Option<&str>,
    _now: Timestamp,
) -> Fleet {
    Fleet {
        rows: Vec::new(),
        home: home.map(str::to_owned),
    }
}

/// The attention line, for the rows `visible` shows, at `width` cells.
#[must_use]
pub fn attention(_fleet: &Fleet, _visible: std::ops::Range<usize>, _width: u16) -> String {
    String::new()
}
