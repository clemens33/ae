//! Who a plain line goes to: the lead unless the lead cannot take it. PURE.
//!
//! The pair is a pair of equals the human does not have to choose between. A
//! line with no `@seat` goes to the lead; when the lead cannot take it the
//! colead does, and the app says so quietly. The verdict is the one `ae list`
//! shows (an agent entry's `alive`, `reason` and observed frame), never a
//! second classifier.

use crate::attention::Reason;
use crate::console::lane::{Item, Kind};
use crate::digest::AgentEntry;
use crate::harness_state::HarnessState;

/// What one seat can take now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Take {
    Yes,
    /// Inside a turn: it takes the line once the turn ends.
    Busy,
    /// Cannot, for this reason.
    No(&'static str),
}

/// A seat's [`Take`] from its entry; a seat the digest does not know is not
/// second-guessed.
pub(super) fn take(agent: Option<&AgentEntry>) -> Take {
    let Some(agent) = agent else {
        return Take::Yes;
    };
    match (agent.alive, agent.reason) {
        (Some(false), _) | (_, Some(Reason::Dead)) => Take::No("not running"),
        (_, Some(Reason::Limit)) => Take::No("out of quota"),
        (_, Some(Reason::Blocked)) => Take::No("blocked"),
        (_, Some(Reason::Throttled)) => Take::No("in error"),
        _ if agent.observed == HarnessState::Busy => Take::Busy,
        _ => Take::Yes,
    }
}

/// The seat holding the human's newest open ask, from the lane as drawn: an
/// ask neither answered, closed nor given up on.
pub(super) fn thread_seat(items: &[Item]) -> Option<&str> {
    let mut open: Vec<(&str, &str)> = Vec::new();
    for item in items {
        match &item.kind {
            Kind::Asked { to, id, .. } => open.push((to, id)),
            Kind::Answer { id, .. } | Kind::Closed { id } | Kind::NotDelivered { id, .. } => {
                open.retain(|(_, asked)| asked != id);
            }
            _ => {}
        }
    }
    open.last().map(|(to, _)| *to)
}

/// Where a plain line goes and what is said about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Route {
    pub(super) seat: String,
    /// The quiet lane note after the send.
    pub(super) note: Option<String>,
    /// The first line of the ask when it was rerouted to a seat that did not
    /// expect it.
    pub(super) prefix: Option<String>,
}

fn because(name: &str, take: Take) -> String {
    match take {
        Take::No(why) => format!("{name} {why}"),
        Take::Busy | Take::Yes => format!("{name} busy"),
    }
}

/// Route a plain line over `pair`, lead first, each with what it can take;
/// `thread` is the seat holding the human's open ask. A seat holding that
/// thread keeps the follow-up even while busy on it; otherwise the lead, else
/// the colead, else (both busy) the lead, which waits for a free composer.
pub(super) fn decide(pair: &[(&str, Take)], thread: Option<&str>) -> Route {
    let route = |seat: &str, note: Option<String>, prefix: Option<String>| Route {
        seat: seat.to_owned(),
        note,
        prefix,
    };
    let Some(&(lead, lead_take)) = pair.first() else {
        return route("", None, None);
    };
    if let Some(&(seat, take)) = pair.iter().find(|(name, _)| Some(*name) == thread)
        && !matches!(take, Take::No(_))
    {
        let note = (seat != lead).then(|| format!("sent to {seat} (your open ask is with {seat})"));
        return route(seat, note, None);
    }
    let Some(&(other, other_take)) = pair.get(1).filter(|_| lead_take != Take::Yes) else {
        return route(lead, None, None);
    };
    let why = because(lead, lead_take);
    match (other_take, lead_take) {
        (Take::Yes, _) => route(
            other,
            Some(format!("sent to {other} ({why})")),
            Some(format!("(routed to you: {why})")),
        ),
        (Take::Busy, Take::No(_)) => route(
            other,
            Some(format!("queued for {other} ({why}, {other} busy)")),
            Some(format!("(routed to you: {why})")),
        ),
        (Take::Busy, _) => route(lead, Some(format!("queued for {lead} (both busy)")), None),
        (Take::No(_), _) => route(
            lead,
            Some(format!(
                "queued for {lead} ({why}, {})",
                because(other, other_take)
            )),
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    //! The decision's oracle is the human's routing ruling: lead by default,
    //! colead when the lead cannot take the line, the open thread kept.

    use super::{Take, decide, take, thread_seat};
    use crate::attention::Reason;
    use crate::console::lane::{Item, Kind};
    use crate::digest::AgentEntry;
    use crate::harness_state::HarnessState;

    fn sent(pair: &[(&str, Take)], thread: Option<&str>) -> (String, Option<String>) {
        let route = decide(pair, thread);
        (route.seat, route.note)
    }

    #[test]
    fn a_seat_is_taken_by_the_verdict_ae_list_shows() {
        let seat = |alive, reason, observed| AgentEntry {
            alive,
            observed,
            reason,
            ..AgentEntry::default()
        };
        let idle = HarnessState::Idle;
        assert_eq!(take(None), Take::Yes);
        assert_eq!(take(Some(&seat(Some(true), None, idle))), Take::Yes);
        assert_eq!(
            take(Some(&seat(Some(false), None, idle))),
            Take::No("not running")
        );
        for (reason, why) in [
            (Reason::Limit, "out of quota"),
            (Reason::Blocked, "blocked"),
            (Reason::Throttled, "in error"),
        ] {
            let down = seat(Some(true), Some(reason), idle);
            assert_eq!(take(Some(&down)), Take::No(why));
        }
        let busy = seat(Some(true), None, HarnessState::Busy);
        assert_eq!(take(Some(&busy)), Take::Busy);
    }

    #[test]
    fn the_lead_takes_a_line_it_can() {
        let pair = [("lead", Take::Yes), ("colead", Take::Yes)];
        assert_eq!(sent(&pair, None), ("lead".to_owned(), None));
    }

    #[test]
    fn a_lead_that_cannot_take_it_hands_the_line_to_a_free_colead() {
        for (take, why) in [
            (Take::Busy, "lead busy"),
            (Take::No("out of quota"), "lead out of quota"),
            (Take::No("not running"), "lead not running"),
        ] {
            let route = decide(&[("lead", take), ("colead", Take::Yes)], None);
            assert_eq!(route.seat, "colead");
            assert_eq!(route.note, Some(format!("sent to colead ({why})")));
            assert_eq!(route.prefix, Some(format!("(routed to you: {why})")));
        }
    }

    #[test]
    fn both_busy_queues_for_the_lead_and_a_busy_colead_beats_a_blocked_lead() {
        let both = [("lead", Take::Busy), ("colead", Take::Busy)];
        assert_eq!(
            sent(&both, None),
            (
                "lead".to_owned(),
                Some("queued for lead (both busy)".to_owned())
            )
        );
        let down = [("lead", Take::No("blocked")), ("colead", Take::Busy)];
        assert_eq!(sent(&down, None).0, "colead");
        let none = [
            ("lead", Take::No("blocked")),
            ("colead", Take::No("not running")),
        ];
        assert_eq!(sent(&none, None).0, "lead");
    }

    #[test]
    fn an_open_ask_keeps_its_follow_up_with_its_seat_even_when_busy() {
        let pair = [("lead", Take::Yes), ("colead", Take::Busy)];
        let (seat, note) = sent(&pair, Some("colead"));
        assert_eq!(seat, "colead");
        assert!(note.is_some_and(|note| note.contains("open ask")));
        let busy = [("lead", Take::Busy), ("colead", Take::Yes)];
        assert_eq!(sent(&busy, Some("lead")), ("lead".to_owned(), None));
        let gone = [("lead", Take::Yes), ("colead", Take::No("not running"))];
        assert_eq!(
            sent(&gone, Some("colead")).0,
            "lead",
            "a seat that cannot take it loses the thread"
        );
    }

    #[test]
    fn the_thread_is_the_newest_ask_nothing_has_closed() {
        let item = |kind| Item {
            micros: 0,
            kind,
            body: String::new(),
            record: None,
        };
        let asked = |to: &str, id: &str| Kind::Asked {
            to: to.to_owned(),
            id: id.to_owned(),
            uncertain: false,
            gap: None,
        };
        let items = vec![
            item(asked("lead", "a")),
            item(asked("colead", "b")),
            item(Kind::Closed { id: "b".to_owned() }),
        ];
        assert_eq!(thread_seat(&items), Some("lead"));
        let answered = [
            items[0].clone(),
            item(Kind::Answer {
                seat: "lead".to_owned(),
                id: "a".to_owned(),
                follow_up: 0,
                late: false,
                gap: None,
                speaker: None,
            }),
        ];
        assert_eq!(thread_seat(&answered), None);
    }
}
