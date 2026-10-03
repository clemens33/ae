//! The Overview tab: the goal, what waits on the human, the latest decision
//! and the latest memo per topic.

use crate::brief::TopicLine;
use crate::console::needs::Section;
use crate::digest::SessionEntry;
use crate::time::Timestamp;

/// The tab's content.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Overview {
    /// The goal and its age.
    pub goal: Option<(String, Option<i64>)>,
    pub open: Vec<Open>,
    /// The latest `decision` memo.
    pub decided: Option<TopicLine>,
    /// The latest memo per topic.
    pub topics: Vec<TopicLine>,
    /// Why the memos could not be read.
    pub memo_gap: Option<String>,
}

/// One thing waiting on the human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    pub seat: String,
    pub text: String,
    pub age_secs: Option<i64>,
}

/// The Overview of `entry`.
#[must_use]
pub fn of(
    _entry: &SessionEntry,
    _needs: Option<&Section>,
    _memo: Result<&[u8], String>,
    _now: Timestamp,
) -> Overview {
    Overview::default()
}
