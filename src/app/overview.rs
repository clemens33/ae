//! The Overview tab: the goal, what waits on the human, the latest decision
//! and the latest memo per topic.
//!
//! PURE. Open is the needs Section the chat shows (so the tab agrees with
//! `ae list`); the memos are `ae brief`'s own fold.

use crate::brief::TopicLine;
use crate::console::needs::{Section, Verdict};
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
    /// The latest memo per topic, newest first, the decision left out.
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

/// The Overview of `entry`: an unreadable memo is named in `memo_gap`, never
/// shown as empty, and Open never depends on it.
#[must_use]
pub fn of(
    entry: &SessionEntry,
    needs: Option<&Section>,
    memo: Result<&[u8], String>,
    now: Timestamp,
) -> Overview {
    let goal = entry
        .goal
        .as_deref()
        .filter(|goal| !goal.trim().is_empty())
        .map(|goal| {
            let age = entry
                .goal_set_epoch
                .map(|epoch| now.epoch().saturating_sub(epoch).max(0));
            (goal.to_owned(), age)
        });
    let open = needs
        .map(|section| section.rows.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|row| match row.verdict {
            Verdict::Reason(reason) => Some(Open {
                seat: row.seat.name.clone(),
                text: super::fleet::asked(&row.detail, reason),
                age_secs: super::fleet::micros_age(row.since_micros, now),
            }),
            Verdict::Unknown(_) => None,
        })
        .collect();
    let (lines, memo_gap) = match memo {
        Ok(bytes) => (crate::brief::topic_lines(bytes, now, None), None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let (decided, topics): (Vec<TopicLine>, Vec<TopicLine>) =
        lines.into_iter().partition(|line| line.topic == "decision");
    Overview {
        goal,
        open,
        decided: decided.into_iter().next(),
        topics,
        memo_gap,
    }
}
