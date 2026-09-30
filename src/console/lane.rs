//! The console lane: what a session's lead pair said to, and needs from, the
//! human, folded from facts other owners already parsed.
//!
//! PURE — no clock, no I/O, no tmux. The journal arrives as parsed
//! [`Event`]s, the transcripts as [`board::Observation`] rows (already past the
//! board's one ae-turn filter, so a marked or harness-wrapped turn never
//! reaches here as a row). Bodies stay RAW: the view routes every field
//! through the board's terminal renderer, the one place a control byte is made
//! inert.

use crate::board::{self, Role};
use crate::events::Event;
use crate::watchdog::{self, HUMAN_BRIDGE_ACTORS, WATCHDOG_ACTOR};
use crate::{reply, send, tracked};

/// One seat of the session's lead pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    /// `main` / `worker.0`.
    pub slot: String,
    /// The seat's name.
    pub name: String,
}

/// What one lane item is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A human turn from a seat's transcript; `generation` > 0 is a predecessor.
    Pane { seat: String, generation: u8 },
    /// A model reply from a seat's transcript (`--all`).
    Assistant { seat: String, generation: u8 },
    /// A chat-bridge message, from the journal, to whichever seat it names.
    Inbound { from: String, to: String },
    /// A reply to a chat-bridge message, from any replier: the journal keeps
    /// only a 600-character summary of its body.
    Reply { from: String, to: String },
    /// A `say` line: a lead-pair seat's name, or `ae` for ae's own notice.
    Said { who: String },
    /// A seat's CURRENT `waiting-user` declaration.
    Card { seat: String },
    /// The watchdog's standing `human-prompt` verdict on a seat.
    NeedsYou { seat: String },
}

/// One lane row: epoch micros (a journal record's second widened), what it
/// is, and its raw body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub micros: i64,
    pub kind: Kind,
    pub body: String,
}

/// The lane: rows oldest first, and one `<actor> — <reason>` per gap.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lane {
    pub items: Vec<Item>,
    pub coverage: Vec<String>,
}

fn item(kind: Kind, micros: i64, body: &str) -> Item {
    Item {
        micros,
        kind,
        body: body.to_owned(),
    }
}

/// A journal record's time in micros: its stamp has second precision.
fn micros(event: &Event) -> i64 {
    event.ts.epoch().saturating_mul(1_000_000)
}

fn bridge(name: &str) -> bool {
    HUMAN_BRIDGE_ACTORS
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Fold one session's lead-pair facts into the lane.
///
/// The journal half is the human-facing record: a chat-bridge thread in FULL
/// (whichever seat it names), `say` lines from the lead pair and ae's own
/// notices, and a worker's `say` only COUNTED — never shown, never silent.
/// Cards and prompts belong to the lead pair alone, and only while the
/// declaration or verdict still stands.
#[must_use]
pub fn fold(
    session: &str,
    seats: &[Seat],
    events: &[Event],
    observation: &board::Observation,
    skipped: usize,
) -> Lane {
    let (mut items, other_say) = journal_items(session, seats, events);
    items.extend(standing_items(session, seats, events));
    let prefix = format!("{session}:");
    for row in &observation.rows {
        let name = row.actor.strip_prefix(&prefix);
        let Some(seat) = name.filter(|name| seats.iter().any(|seat| seat.name == *name)) else {
            continue;
        };
        let (seat, generation) = (seat.to_owned(), row.generation);
        let kind = match row.role {
            Role::Human => Kind::Pane { seat, generation },
            Role::Assistant => Kind::Assistant { seat, generation },
        };
        items.push(item(kind, row.ts, &row.body));
    }
    items.sort_by_key(|item| item.micros);
    let mut coverage: Vec<String> = observation
        .coverage
        .iter()
        .map(|gap| format!("{} — {}", gap.actor, gap.reason))
        .collect();
    for (who, count, noun, tail) in [
        ("journal", skipped, "unreadable line", "skipped"),
        ("say", other_say, "line", "from other seats not shown"),
    ] {
        let plural = if count == 1 { "" } else { "s" };
        if count > 0 {
            coverage.push(format!("{who} — {count} {noun}{plural} {tail}"));
        }
    }
    Lane { items, coverage }
}

/// The chat-bridge thread and the `say` lines, plus how many worker `say`
/// lines were left out.
fn journal_items(session: &str, seats: &[Seat], events: &[Event]) -> (Vec<Item>, usize) {
    let (mut items, mut other_say) = (Vec::new(), 0);
    for event in events {
        let (actor, action) = (event.actor.clone(), event.action.as_str());
        let to = event.target.clone().unwrap_or_default();
        let asks = [
            send::ACTION,
            tracked::Kind::Ask.action(),
            tracked::Kind::Review.action(),
        ];
        let kind = if bridge(&actor) && asks.contains(&action) {
            Kind::Inbound { from: actor, to }
        } else if action == reply::ACTION && bridge(&to) {
            Kind::Reply { from: actor, to }
        } else if action == "chat" {
            let seat = seats
                .iter()
                .find(|seat| watchdog::event_is_actor(event, session, &seat.slot, &seat.name));
            if let Some(seat) = seat {
                Kind::Said {
                    who: seat.name.clone(),
                }
            } else if actor == WATCHDOG_ACTOR || actor.starts_with("ae:") {
                Kind::Said {
                    who: "ae".to_owned(),
                }
            } else {
                other_say += 1;
                continue;
            }
        } else {
            continue;
        };
        items.push(item(
            kind,
            micros(event),
            event.summary.as_deref().unwrap_or(""),
        ));
    }
    (items, other_say)
}

/// What a lead-pair seat still asks of the human: its CURRENT `waiting-user`
/// declaration, and the watchdog's uncleared `human-prompt`.
fn standing_items(session: &str, seats: &[Seat], events: &[Event]) -> Vec<Item> {
    let mut items = Vec::new();
    for seat in seats {
        let seat_name = seat.name.clone();
        let card = watchdog::latest_relevant_event(events, session, &seat.slot, &seat.name)
            .filter(watchdog::declaration_current)
            .map(|relevant| relevant.event)
            .filter(|event| event.declared_state() == Some("waiting-user"));
        let prompt = events
            .iter()
            .rev()
            .find(|event| {
                event.actor == WATCHDOG_ACTOR
                    && ["human-prompt", "human-prompt-cleared"].contains(&event.action.as_str())
                    && watchdog::event_is_addressed_to(event, session, &seat.slot, &seat.name)
            })
            .filter(|event| event.action == "human-prompt");
        let card = card.map(|event| {
            (
                event,
                Kind::Card {
                    seat: seat_name.clone(),
                },
            )
        });
        let prompt = prompt.map(|event| (event, Kind::NeedsYou { seat: seat_name }));
        for (event, kind) in card.into_iter().chain(prompt) {
            items.push(item(
                kind,
                micros(event),
                event.summary.as_deref().unwrap_or(""),
            ));
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::{Kind, Lane, Seat, fold};
    use crate::board::{Coverage, Observation, Role, Row};
    use crate::events::Event;
    use crate::tool::ToolKind;
    use std::fmt::Write as _;

    const S: &str = "aedev";
    const T0: &str = "2026-09-30T06:00:00Z";
    const T1: &str = "2026-09-30T06:01:00Z";
    const T2: &str = "2026-09-30T06:02:00Z";
    const MICROS_T1: i64 = 1_790_748_060_000_000;

    fn ev(ts: &str, actor: &str, action: &str, to: &str, text: &str) -> Event {
        let mut line = format!(r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}""#);
        let reference = if action == "state" { text } else { "r-1" };
        for (key, value) in [("target", to), ("ref", reference), ("summary", text)] {
            let _ = write!(line, r#","{key}":"{value}""#);
        }
        Event::parse_line(&format!("{line}}}")).unwrap()
    }

    fn row(actor: &str, role: Role, generation: u8, ts: i64, body: &str) -> Row {
        Row {
            ts,
            actor: format!("{S}:{actor}"),
            role,
            body: body.to_owned(),
            source: ToolKind::Claude,
            file: "f".to_owned(),
            offset: u64::try_from(ts).unwrap(),
            generation,
        }
    }

    fn pair() -> [Seat; 2] {
        [("main", "lead"), ("worker.0", "colead")].map(|(slot, name)| Seat {
            slot: slot.to_owned(),
            name: name.to_owned(),
        })
    }

    /// Each item as `kind body`, in lane order.
    fn shown(events: &[Event], rows: Vec<Row>) -> Vec<String> {
        let observation = Observation {
            rows,
            ..Observation::default()
        };
        let lane = fold(S, &pair(), events, &observation, 0);
        lane.items
            .iter()
            .map(|item| {
                let tag = match &item.kind {
                    Kind::Pane { seat, generation } => format!("pane {seat}#{generation}"),
                    Kind::Assistant { seat, generation } => format!("asst {seat}#{generation}"),
                    Kind::Inbound { from, to } => format!("in {from}>{to}"),
                    Kind::Reply { from, to } => format!("reply {from}>{to}"),
                    Kind::Said { who } => format!("said {who}"),
                    Kind::Card { seat } => format!("card {seat}"),
                    Kind::NeedsYou { seat } => format!("needs {seat}"),
                };
                format!("{tag}: {}", item.body)
            })
            .collect()
    }

    #[test]
    fn transcript_rows_of_the_lead_pair_are_pane_and_assistant_items_and_others_never() {
        let rows = vec![
            row("lead", Role::Human, 0, MICROS_T1, "typed in lead"),
            row(
                "colead",
                Role::Assistant,
                2,
                MICROS_T1 + 1,
                "colead answers",
            ),
            row(
                "scout",
                Role::Human,
                0,
                MICROS_T1 + 2,
                "a worker is not mirrored",
            ),
        ];
        assert_eq!(
            shown(&[], rows),
            [
                "pane lead#0: typed in lead",
                "asst colead#2: colead answers"
            ]
        );
    }

    #[test]
    fn a_bridge_thread_is_shown_whole_and_only_the_human_facing_journal_rows_are() {
        let events = [
            ev(T0, "telegram:42", "send", "lead", "ship it?"),
            ev(T0, "discord:7", "ask", "scout", "worker thread too"),
            ev(T1, "lead", "reply", "telegram:42", "yes, shipped"),
            ev(T1, "scout", "reply", "discord:7", "worker answers"),
            ev(T1, "lead", "send", "colead", "peer chatter"),
            ev(T2, "lead", "reply", "ae:compact:u", "not the human"),
        ];
        assert_eq!(
            shown(&events, Vec::new()),
            [
                "in telegram:42>lead: ship it?",
                "in discord:7>scout: worker thread too",
                "reply lead>telegram:42: yes, shipped",
                "reply scout>discord:7: worker answers",
            ]
        );
    }

    #[test]
    fn say_lines_are_the_lead_pairs_and_ae_own_and_a_workers_are_counted_not_shown() {
        let events = [
            ev(T0, "colead", "chat", "", "announcing"),
            ev(T0, "watchdog", "chat", "", "moved lead to another profile"),
            ev(T1, "scout", "chat", "", "a worker says"),
            ev(T1, "scout", "chat", "", "and again"),
        ];
        let observation = Observation::default();
        let lane = fold(S, &pair(), &events, &observation, 0);
        assert_eq!(lane.coverage, ["say — 2 lines from other seats not shown"]);
        assert_eq!(
            shown(&events, Vec::new()),
            [
                "said colead: announcing",
                "said ae: moved lead to another profile"
            ]
        );
    }

    #[test]
    fn a_waiting_user_declaration_is_a_card_only_while_it_is_current() {
        let waiting = |ts: &str, actor: &str, state: &str| ev(ts, actor, "state", "", state);
        let asks = waiting(T0, "lead", "waiting-user");
        assert_eq!(
            shown(std::slice::from_ref(&asks), Vec::new()),
            ["card lead: waiting-user"]
        );
        for later in ["working", "done"] {
            let events = [asks.clone(), waiting(T1, "lead", later)];
            assert!(shown(&events, Vec::new()).is_empty(), "{later}");
        }
        assert!(shown(&[waiting(T0, "lead", "waiting-agent")], Vec::new()).is_empty());
        assert!(shown(&[waiting(T0, "scout", "waiting-user")], Vec::new()).is_empty());
    }

    #[test]
    fn a_human_prompt_is_a_needs_you_item_until_the_watchdog_clears_it() {
        let raised = ev(
            T0,
            "watchdog",
            "human-prompt",
            "colead",
            "Trust this folder?",
        );
        let cleared = ev(T1, "watchdog", "human-prompt-cleared", "colead", "gone");
        assert_eq!(
            shown(std::slice::from_ref(&raised), Vec::new()),
            ["needs colead: Trust this folder?"]
        );
        assert!(shown(&[raised, cleared], Vec::new()).is_empty());
    }

    #[test]
    fn items_merge_in_time_order_and_every_gap_is_a_coverage_row() {
        let events = [ev(T2, "colead", "chat", "", "later")];
        let observation = Observation {
            rows: vec![row("lead", Role::Human, 0, MICROS_T1, "earlier")],
            coverage: vec![Coverage {
                actor: format!("{S}:lead"),
                reason: "torn last record".to_owned(),
            }],
            ..Observation::default()
        };
        let lane: Lane = fold(S, &pair(), &events, &observation, 1);
        let bodies: Vec<&str> = lane.items.iter().map(|item| item.body.as_str()).collect();
        assert_eq!(bodies, ["earlier", "later"]);
        assert_eq!(
            lane.coverage,
            [
                "aedev:lead — torn last record",
                "journal — 1 unreadable line skipped"
            ]
        );
    }
}
