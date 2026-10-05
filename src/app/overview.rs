//! The Overview tab: the goal, what waits on the human, the latest decision
//! and the latest memo per topic.
//!
//! PURE. Open is the needs Section the chat shows (so the tab agrees with
//! `ae list`); the memos are `ae brief`'s own fold.

use crate::brief::{Filed, TopicLine};
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
    match memo {
        Ok(bytes) => of_filed(entry, needs, Ok(&crate::brief::filed(bytes)), now),
        Err(why) => of_filed(entry, needs, Err(why), now),
    }
}

/// [`of`] from a memo [`crate::brief::filed`] already read: every age is
/// judged at `now`, so a memo read earlier shows its age as of the show.
#[must_use]
pub fn of_filed(
    entry: &SessionEntry,
    needs: Option<&Section>,
    memo: Result<&[Filed], String>,
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
        Ok(filed) => (crate::brief::aged(filed, now, None), None),
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

#[cfg(test)]
mod tests {
    use super::{Overview, of, of_filed};
    use crate::digest::{SessionEntry, Status};
    use crate::time::Timestamp;

    /// A memo read once is aged at each show: the same read shown an hour
    /// later is an hour older, newest topic first, an unreadable stamp last,
    /// exactly as the bytes read at that show would say.
    #[test]
    fn a_memo_read_earlier_is_aged_at_the_show() {
        let file = concat!(
            "2026-09-06T11:00:00Z\tcl:lead\tdecision\troute review\n",
            "not a stamp\tcl:lead\tlost\tno age\n",
            "2026-09-06T11:50:00Z\tcl:brief\tparking\tresume here\n",
            "2026-09-06T11:30:00Z\tcl:lead\tdecision\tgate once\n",
        );
        let filed = crate::brief::filed(file.as_bytes());
        let entry = SessionEntry::new("api", Status::Running);
        let at = |stamp| Timestamp::parse(stamp).expect("a timestamp");
        let ages = |overview: &Overview| -> Vec<(String, Option<i64>)> {
            overview
                .topics
                .iter()
                .map(|line| (line.topic.clone(), line.age_secs))
                .collect()
        };
        let read = of_filed(&entry, None, Ok(&filed), at("2026-09-06T12:00:00Z"));
        assert_eq!(
            ages(&read),
            [("parking".to_owned(), Some(600)), ("lost".to_owned(), None)]
        );
        let shown = at("2026-09-06T13:00:00Z");
        let later = of_filed(&entry, None, Ok(&filed), shown);
        assert_eq!(
            ages(&later),
            [
                ("parking".to_owned(), Some(4_200)),
                ("lost".to_owned(), None)
            ]
        );
        let decided = later.decided.clone().expect("the latest decision");
        assert_eq!(
            (decided.text.as_str(), decided.age_secs),
            ("gate once", Some(5_400))
        );
        assert_eq!(later, of(&entry, None, Ok(file.as_bytes()), shown));
    }
}
