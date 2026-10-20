//! The sidebar's rows: one per session, in fleet order, two lines each.
//!
//! PURE. Every fact comes from an owner that already computed it: the world
//! `ae list` reads, the picker's seat facts, the needs Section the chat shows,
//! the fleet order the status strip draws.

use std::collections::BTreeMap;

use crate::attention::Reason;
use crate::console::needs::{Section, Verdict};
use crate::digest::{SessionEntry, Status};
use crate::listing::World;
use crate::theme::{FleetOrder, FleetRow, Mark};
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
    /// The session this app was opened for, when the fleet holds it.
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
    /// Open requests past the unanswered threshold and not accepted; `None`
    /// when the journal read was not complete.
    pub unanswered: Option<usize>,
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

impl Fleet {
    /// These rows in the order `names` lists them, any row it omits after them
    /// in its own order, `index` renumbered to the new places.
    #[must_use]
    pub fn arranged(mut self, names: &[String]) -> Self {
        self.rows.sort_by_key(|row| {
            names
                .iter()
                .position(|name| *name == row.name)
                .unwrap_or(usize::MAX)
        });
        for (at, row) in self.rows.iter_mut().enumerate() {
            row.index = at + 1;
        }
        self
    }
}

/// The sidebar's rows: running sessions with the orchestrator first, then the
/// human's `fleet_order` pins, then the rest by the newest AI answer to the
/// human, newest first (`answer_epoch`); a session never answered after them
/// in creation order; then stopped ones as the picker sorts them — dated by
/// `last_live` newest first, undated after, ties by name.
#[must_use]
pub fn rows(
    world: &World,
    facts: &BTreeMap<String, Facts>,
    last_live: &BTreeMap<String, i64>,
    order: &FleetOrder,
    needs: &BTreeMap<String, Section>,
    home: Option<&str>,
    now: Timestamp,
) -> Fleet {
    let (stopped, running): (Vec<&SessionEntry>, Vec<&SessionEntry>) = world
        .sessions
        .iter()
        .partition(|entry| entry.status == Status::Stopped);
    let strip: Vec<FleetRow> = running
        .iter()
        .map(|entry| FleetRow {
            name: entry.name.clone(),
            id: match facts.get(&entry.name) {
                Some(Facts::Seats { id, .. }) => id.clone(),
                _ => String::new(),
            },
            mark: Mark::Idle,
            current: false,
        })
        .collect();
    let mut stopped = stopped;
    stopped.sort_by(|left, right| {
        let moment = |entry: &SessionEntry| last_live.get(&entry.name).copied();
        moment(right)
            .cmp(&moment(left))
            .then_with(|| left.name.cmp(&right.name))
    });
    let (pinned, mut rest): (Vec<&SessionEntry>, Vec<&SessionEntry>) =
        crate::theme::ordered_fleet_rows(&strip, order)
            .into_iter()
            .filter_map(|row| running.iter().find(|entry| entry.name == row.name))
            .partition(|entry| {
                entry.name == crate::orchestrator::ORCHESTRATOR_SESSION
                    || order.place(&entry.name) != usize::MAX
            });
    rest.sort_by_key(|entry| std::cmp::Reverse(entry.answer_epoch));
    let ordered = pinned
        .into_iter()
        .chain(rest)
        .chain(stopped.iter().copied());
    let empty = Section::default();
    let rows: Vec<Row> = ordered
        .enumerate()
        .map(|(at, entry)| {
            let section = needs.get(&entry.name).unwrap_or(&empty);
            row(
                entry,
                facts.get(&entry.name),
                last_live,
                section,
                at + 1,
                now,
            )
        })
        .map(|mut row| {
            row.home = home == Some(row.name.as_str());
            row
        })
        .collect();
    let home = home
        .filter(|home| rows.iter().any(|row| row.name == *home))
        .map(str::to_owned);
    Fleet { rows, home }
}

/// One session's row.
fn row(
    entry: &SessionEntry,
    facts: Option<&Facts>,
    last_live: &BTreeMap<String, i64>,
    section: &Section,
    index: usize,
    now: Timestamp,
) -> Row {
    let stopped = entry.status == Status::Stopped;
    let agents = match facts {
        Some(Facts::Seats { agents, .. }) if !stopped => Some(agents.as_slice()),
        _ => None,
    };
    let mark = if stopped {
        Mark::Dead
    } else if entry.attention.is_some() {
        // Aggregated as MARKS: a stale seat's severity cannot hide another
        // seat that needs the human.
        Mark::for_reasons(
            entry
                .agents
                .iter()
                .filter_map(|agent| agent.reason)
                .chain(entry.attention),
        )
    } else {
        match agents {
            Some(agents) if agents.iter().any(|agent| agent.mark() == Mark::Working) => {
                Mark::Working
            }
            Some(_) => Mark::Idle,
            None => Mark::Stale,
        }
    };
    let counts = match (stopped, agents) {
        (true, _) => Counts::Stopped,
        (false, Some(agents)) => Counts::Marks(tally(agents)),
        (false, None) => Counts::Unknown,
    };
    let line2 = if stopped {
        Line2::NotRunning {
            seats: entry.agents.len(),
            stopped_secs: last_live
                .get(&entry.name)
                .map(|epoch| now.epoch().saturating_sub(*epoch).max(0)),
        }
    } else {
        line_two(entry, section, now)
    };
    Row {
        name: entry.name.clone(),
        index,
        mark,
        needy: section
            .rows
            .iter()
            .any(|row| matches!(row.verdict, Verdict::Reason(_))),
        unanswered: entry.unanswered,
        counts,
        line2,
        home: false,
    }
}

/// The seats' marks in urgency order, a blocked seat apart from the rest of
/// its mark, non-zero only.
fn tally(agents: &[PickerAgent]) -> Vec<(Mark, bool, usize)> {
    Mark::BY_URGENCY
        .into_iter()
        .flat_map(|mark| [(mark, false), (mark, true)])
        .filter_map(|(mark, blocked)| {
            let count = agents
                .iter()
                .filter(|agent| agent.mark() == mark && (agent.state == "blocked") == blocked)
                .count();
            (count > 0).then_some((mark, blocked, count))
        })
        .collect()
}

/// A running session's line 2: the open question, else a seat down, else the
/// goal, else nothing to say.
fn line_two(entry: &SessionEntry, section: &Section, now: Timestamp) -> Line2 {
    let reasons = || {
        section.rows.iter().filter_map(|row| match row.verdict {
            Verdict::Reason(reason) => Some((row, reason)),
            Verdict::Unknown(_) => None,
        })
    };
    if let Some((row, reason)) = reasons().find(|(_, reason)| *reason != Reason::Dead) {
        return Line2::Question {
            text: asked(&row.detail, reason),
            age_secs: micros_age(row.since_micros, now),
        };
    }
    if reasons().any(|(_, reason)| reason == Reason::Dead) {
        return Line2::SeatDown;
    }
    match entry.goal.as_deref().filter(|goal| !goal.trim().is_empty()) {
        Some(goal) => Line2::Goal {
            text: goal.to_owned(),
            age_secs: entry
                .goal_set_epoch
                .map(|epoch| now.epoch().saturating_sub(epoch).max(0)),
        },
        None => Line2::NoGoal,
    }
}

/// What a needs row asks: its record's words, or the reason's own word when
/// the record says nothing.
pub(crate) fn asked(detail: &str, reason: Reason) -> String {
    if detail.trim().is_empty() {
        reason.as_str().to_owned()
    } else {
        detail.to_owned()
    }
}

/// Seconds from an epoch-micros stamp to `now`, never negative.
pub(crate) fn micros_age(micros: Option<i64>, now: Timestamp) -> Option<i64> {
    micros.map(|micros| now.epoch().saturating_sub(micros / 1_000_000).max(0))
}

/// The whole fleet in numbers for the header; `unknown` makes `agents` and
/// `working` lower bounds (a running session published no seat facts).
#[derive(Debug, Default)]
pub(super) struct Summary {
    pub agents: usize,
    pub working: usize,
    pub needy: usize,
    pub unknown: bool,
}

/// The fleet's [`Summary`], counted from each row's seat counts, never its mark.
pub(super) fn summary(fleet: &Fleet) -> Summary {
    let mut sum = Summary::default();
    for row in &fleet.rows {
        sum.needy += usize::from(row.needy);
        match (&row.counts, &row.line2) {
            (Counts::Marks(marks), _) => marks.iter().for_each(|(mark, _, count)| {
                sum.agents = sum.agents.saturating_add(*count);
                sum.working = sum
                    .working
                    .saturating_add(count * usize::from(*mark == Mark::Working));
            }),
            (Counts::Stopped, Line2::NotRunning { seats, .. }) => {
                sum.agents = sum.agents.saturating_add(*seats);
            }
            (Counts::Unknown, _) => sum.unknown = true,
            (Counts::Stopped, _) => {}
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    //! Mutation pins (pins-plan.md #63/#71/#73). Oracles: the `app_spec` model
    //! rule (a home with no row is no home) and the attention-line contract
    //! (plain-word location only after a needy row scrolled out of view).

    use std::collections::BTreeMap;

    use super::{Counts, Fleet, Line2, Row, rows};
    use crate::digest::{SessionEntry, Status};
    use crate::listing::World;
    use crate::theme::{FleetOrder, Mark};
    use crate::time::Timestamp;

    fn fleet_of(names: &[&str], home: &str) -> Fleet {
        let now = Timestamp::from_epoch(1_759_500_600);
        let sessions = names
            .iter()
            .map(|name| SessionEntry::new(*name, Status::Running))
            .collect();
        rows(
            &World::new(now, sessions),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &FleetOrder::EMPTY,
            &BTreeMap::new(),
            Some(home),
            now,
        )
    }

    /// Reordering by a name list: listed rows in its order, unlisted after in
    /// their own, every `index` renumbered.
    #[test]
    fn arranging_follows_the_names_and_renumbers() {
        let row = |name: &str, index| Row {
            name: name.to_owned(),
            index,
            mark: Mark::Idle,
            needy: false,
            unanswered: None,
            counts: Counts::Unknown,
            line2: Line2::NoGoal,
            home: false,
        };
        let fleet = Fleet {
            rows: vec![row("a", 1), row("b", 2), row("c", 3), row("d", 4)],
            home: None,
        };
        let arranged = fleet.arranged(&["c".to_owned(), "a".to_owned()]);
        let seen: Vec<(&str, usize)> = arranged
            .rows
            .iter()
            .map(|row| (row.name.as_str(), row.index))
            .collect();
        assert_eq!(seen, [("c", 1), ("a", 2), ("b", 3), ("d", 4)]);
    }

    #[test]
    fn home_is_kept_only_when_its_session_has_a_row() {
        assert_eq!(fleet_of(&["api"], "api").home.as_deref(), Some("api"));
        assert_eq!(fleet_of(&["web"], "api").home, None);
    }
}
