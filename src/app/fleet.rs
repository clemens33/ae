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

/// The sidebar's rows: running sessions in the status strip's order (the
/// orchestrator first, then the human's `fleet_order`, then creation), then
/// stopped ones as the picker sorts them — dated by `last_live` newest first,
/// undated after, ties by name.
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
    let ordered = crate::theme::ordered_fleet_rows(&strip, order)
        .into_iter()
        .filter_map(|row| running.iter().find(|entry| entry.name == row.name))
        .chain(stopped.iter());
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
    } else if let Some(reason) = entry.attention {
        Mark::for_reason(reason)
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

/// The attention line, for the rows `visible` (positions in `fleet.rows`)
/// shows, at `width` cells: every needy session named in fleet order, a `↑`
/// or `↓` after one scrolled out of view, compact when the words do not fit.
#[must_use]
pub fn attention(fleet: &Fleet, visible: std::ops::Range<usize>, width: u16) -> String {
    let needy: Vec<(usize, &Row)> = fleet
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.needy)
        .collect();
    let arrow = |at: usize| {
        if at < visible.start {
            "↑"
        } else if at >= visible.end {
            "↓"
        } else {
            ""
        }
    };
    let glyph = |row: &Row| row.mark.glyph(true);
    match needy.as_slice() {
        [] => "Nothing needs you.".to_owned(),
        [(at, row)] => format!("{} {}{} needs you", glyph(row), row.name, arrow(*at)),
        many => {
            let full: Vec<String> = many
                .iter()
                .map(|(at, row)| format!("{} {}{}", glyph(row), row.name, arrow(*at)))
                .collect();
            let full = format!("{} need you  {}", many.len(), full.join("  "));
            if full.chars().count() <= usize::from(width) {
                return full;
            }
            let compact: Vec<String> = many
                .iter()
                .map(|(at, row)| format!("{}{}{}", glyph(row), row.name, arrow(*at)))
                .collect();
            format!("{} need {}", many.len(), compact.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    //! Mutation pins (pins-plan.md #63/#71/#73). Oracles: the `app_spec` model
    //! rule (a home with no row is no home) and the attention-line contract
    //! (plain-word location only after a needy row scrolled out of view).

    use std::collections::BTreeMap;

    use super::{Counts, Fleet, Line2, Row, attention, rows};
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

    #[test]
    fn home_is_kept_only_when_its_session_has_a_row() {
        assert_eq!(fleet_of(&["api"], "api").home.as_deref(), Some("api"));
        assert_eq!(fleet_of(&["web"], "api").home, None);
    }

    fn needy_at(needy: usize) -> Fleet {
        let row = |at: usize| Row {
            name: format!("s{at}"),
            index: at + 1,
            mark: Mark::NeedsYou,
            needy: at == needy,
            counts: Counts::Marks(vec![(Mark::NeedsYou, false, 1)]),
            line2: Line2::NoGoal,
            home: false,
        };
        Fleet {
            rows: (0..5).map(row).collect(),
            home: None,
        }
    }

    #[test]
    fn a_needy_row_above_the_view_carries_a_plain_location() {
        assert_eq!(attention(&needy_at(0), 1..4, 44), "s0 needs you · above");
        assert_eq!(attention(&needy_at(1), 1..4, 44), "s1 needs you");
    }
}
