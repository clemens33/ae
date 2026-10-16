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
pub(crate) enum Take {
    Yes,
    /// Inside a turn: it takes the line once the turn ends.
    Busy,
    /// Cannot, for this reason.
    No(&'static str),
}

/// A seat's [`Take`] from its entry; a seat the digest does not know is not
/// second-guessed.
pub(crate) fn take(agent: Option<&AgentEntry>) -> Take {
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
pub(crate) fn thread_seat(items: &[Item]) -> Option<&str> {
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
pub(crate) struct Route {
    pub(crate) seat: String,
    /// The quiet lane note after the send.
    pub(crate) note: Option<String>,
    /// The first line of the ask when it was rerouted to a seat that did not
    /// expect it.
    pub(crate) prefix: Option<String>,
}

fn because(name: &str, take: Take) -> String {
    match take {
        Take::No(why) => format!("{name} {why}"),
        Take::Busy | Take::Yes => format!("{name} busy"),
    }
}

/// A seat is pressed once its own account's judged quota reads this many
/// percent: the quota ladder's 80% step-down line.
const PRESSED_AT: f64 = 80.0;

/// The colead takes a line off a pressed lead only when its own quota reads at
/// least this many points lower.
const HEADROOM_GAP: f64 = 20.0;

/// Route a plain line over `pair`, lead first, each with what it can take and
/// its judged quota percentage (`quota`, index-aligned with `pair`; a missing
/// or `None` entry is unknown and is no evidence); `thread` is the seat
/// holding the human's open ask. A seat holding that thread keeps the
/// follow-up even while busy on it; otherwise the lead, else the colead, else
/// (both busy) the lead, which waits for a free composer. With both free the
/// lead keeps the line unless its quota is pressed and the colead's is clearly
/// lower.
pub(crate) fn decide(pair: &[(&str, Take)], quota: &[Option<f64>], thread: Option<&str>) -> Route {
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
    let used = |at: usize| quota.get(at).copied().flatten();
    if lead_take == Take::Yes
        && let Some(&(other, Take::Yes)) = pair.get(1)
        && let (Some(lead_used), Some(other_used)) = (used(0), used(1))
        && lead_used >= PRESSED_AT
        && lead_used - other_used >= HEADROOM_GAP
    {
        let why = format!("{lead} quota {:.0}%", lead_used.round());
        return route(
            other,
            Some(format!("sent to {other} ({why})")),
            Some(format!("(routed to you: {why})")),
        );
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
        let route = decide(pair, &[], thread);
        (route.seat, route.note)
    }

    /// Both seats free; `lead` and `colead` are their judged quota percentages
    /// (`None` = unknown). Where the line goes and what is said about it.
    fn weighed(lead: Option<f64>, colead: Option<f64>) -> super::Route {
        let pair = [("lead", Take::Yes), ("colead", Take::Yes)];
        decide(&pair, &[lead, colead], None)
    }

    /// The quota ruling: with both seats free and no open ask, the lead keeps
    /// the line unless its own account reads 80% or more and the colead's
    /// reads at least 20 points lower. The numbers below are the ruling's, not
    /// the constants'.
    #[test]
    fn a_pressed_lead_hands_the_line_to_a_colead_with_clearly_more_room() {
        let route = weighed(Some(86.0), Some(60.0));
        assert_eq!(route.seat, "colead");
        assert_eq!(
            route.note.as_deref(),
            Some("sent to colead (lead quota 86%)")
        );
        assert_eq!(
            route.prefix.as_deref(),
            Some("(routed to you: lead quota 86%)")
        );
    }

    #[test]
    fn the_thresholds_are_80_percent_pressure_and_a_20_point_gap_both_inclusive() {
        let to = |lead, colead| weighed(Some(lead), Some(colead)).seat;
        assert_eq!(to(79.0, 0.0), "lead", "79% is not pressed");
        assert_eq!(to(80.0, 0.0), "colead", "80% is pressed");
        assert_eq!(to(80.0, 60.0), "colead", "a gap of exactly 20 points");
        assert_eq!(to(80.0, 61.0), "lead", "a gap of 19 points");
        assert_eq!(to(100.0, 80.0), "colead", "20 points at the top");
        assert_eq!(to(90.0, 71.0), "lead", "19 points at 90%");
        assert_eq!(to(80.0, 100.0), "lead", "a fuller colead never takes it");
    }

    #[test]
    fn a_colead_that_is_as_pressed_or_more_never_takes_the_line_off_the_lead() {
        for (lead, colead) in [(95.0, 95.0), (85.0, 99.0), (30.0, 95.0), (10.0, 10.0)] {
            let route = weighed(Some(lead), Some(colead));
            assert_eq!(route.seat, "lead", "lead {lead}% colead {colead}%");
            assert_eq!((route.note, route.prefix), (None, None));
        }
    }

    #[test]
    fn unknown_quota_is_no_evidence_and_the_lead_keeps_the_line() {
        for (lead, colead) in [
            (None, None),
            (None, Some(0.0)),
            (Some(99.0), None),
            (Some(f64::NAN), Some(0.0)),
            (Some(99.0), Some(f64::NAN)),
        ] {
            let route = weighed(lead, colead);
            assert_eq!(route.seat, "lead", "lead {lead:?} colead {colead:?}");
            assert_eq!((route.note, route.prefix), (None, None));
        }
        let pair = [("lead", Take::Yes), ("colead", Take::Yes)];
        for short in [&[][..], &[Some(99.0)][..]] {
            assert_eq!(decide(&pair, short, None).seat, "lead", "no colead reading");
        }
    }

    #[test]
    fn the_note_names_the_seats_and_the_lead_percentage_as_it_reads() {
        let pair = [("main", Take::Yes), ("second", Take::Yes)];
        let route = decide(&pair, &[Some(80.4), Some(10.0)], None);
        assert_eq!(route.seat, "second");
        assert_eq!(
            route.note.as_deref(),
            Some("sent to second (main quota 80%)")
        );
        assert_eq!(
            route.prefix.as_deref(),
            Some("(routed to you: main quota 80%)")
        );
    }

    /// The percentage in the note is the lead's, rounded to a whole number
    /// with halves going up.
    #[test]
    fn the_percentage_in_the_note_rounds_half_up() {
        let note = |lead| weighed(Some(lead), Some(0.0)).note.expect("rerouted");
        assert_eq!(note(86.5), "sent to colead (lead quota 87%)");
        assert_eq!(note(86.4), "sent to colead (lead quota 86%)");
        assert_eq!(note(99.6), "sent to colead (lead quota 100%)");
    }

    /// A pair of one has nobody to hand the line to; readings beyond the pair
    /// name no seat and are ignored.
    #[test]
    fn a_lone_seat_keeps_the_line_and_readings_past_the_pair_are_ignored() {
        let solo = [("lead", Take::Yes)];
        assert_eq!(
            sent_with(&solo, &[Some(95.0)], None),
            ("lead".to_owned(), None)
        );
        let pair = [("lead", Take::Yes), ("colead", Take::Yes)];
        assert_eq!(
            sent_with(&pair, &[Some(95.0), Some(95.0), Some(0.0)], None),
            ("lead".to_owned(), None),
            "a third reading is no colead"
        );
        assert_eq!(decide(&[], &[Some(95.0), Some(0.0)], None).seat, "");
    }

    /// Busy and limit rules decide first: quota only breaks the tie between
    /// two seats that can both take the line.
    #[test]
    fn a_seat_that_cannot_take_the_line_outranks_the_quota() {
        let quota = [Some(10.0), Some(99.0)];
        let busy_lead = [("lead", Take::Busy), ("colead", Take::Yes)];
        let route = decide(&busy_lead, &quota, None);
        assert_eq!(
            route.seat, "colead",
            "a busy lead hands over whatever the quota"
        );
        assert_eq!(route.note.as_deref(), Some("sent to colead (lead busy)"));
        let out = [("lead", Take::No("out of quota")), ("colead", Take::Yes)];
        let route = decide(&out, &quota, None);
        assert_eq!(
            route.note.as_deref(),
            Some("sent to colead (lead out of quota)")
        );
        let pressed = [Some(95.0), Some(0.0)];
        let busy_colead = [("lead", Take::Yes), ("colead", Take::Busy)];
        assert_eq!(
            sent_with(&busy_colead, &pressed, None),
            ("lead".to_owned(), None),
            "a busy colead is not room"
        );
        let down_colead = [("lead", Take::Yes), ("colead", Take::No("blocked"))];
        assert_eq!(
            sent_with(&down_colead, &pressed, None),
            ("lead".to_owned(), None)
        );
        let both_busy = [("lead", Take::Busy), ("colead", Take::Busy)];
        assert_eq!(
            sent_with(&both_busy, &pressed, None),
            (
                "lead".to_owned(),
                Some("queued for lead (both busy)".to_owned())
            )
        );
    }

    #[test]
    fn an_open_ask_keeps_its_seat_whatever_the_quota() {
        let pair = [("lead", Take::Yes), ("colead", Take::Yes)];
        let pressed_lead = [Some(95.0), Some(0.0)];
        assert_eq!(
            sent_with(&pair, &pressed_lead, Some("lead")),
            ("lead".to_owned(), None),
            "the thread seat keeps its follow-up even pressed"
        );
        let pressed_colead = [Some(0.0), Some(95.0)];
        let (seat, note) = sent_with(&pair, &pressed_colead, Some("colead"));
        assert_eq!(seat, "colead");
        assert!(note.is_some_and(|note| note.contains("open ask")));
        let gone = [("lead", Take::Yes), ("colead", Take::No("not running"))];
        assert_eq!(
            sent_with(&gone, &pressed_colead, Some("colead")).0,
            "lead",
            "a thread seat that cannot take it is still lost"
        );
        // A thread seat that cannot take it frees the quota to decide again.
        let freed = [("lead", Take::Yes), ("colead", Take::Yes)];
        assert_eq!(sent_with(&freed, &pressed_lead, Some("other")).0, "colead");
    }

    fn sent_with(
        pair: &[(&str, Take)],
        quota: &[Option<f64>],
        thread: Option<&str>,
    ) -> (String, Option<String>) {
        let route = decide(pair, quota, thread);
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
            let route = decide(&[("lead", take), ("colead", Take::Yes)], &[], None);
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
