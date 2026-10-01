//! The console lane: what a session's lead pair said to, and needs from, the
//! human, folded from facts other owners already parsed.
//!
//! PURE — no clock, no I/O, no tmux. The journal arrives as parsed
//! [`Event`]s, the transcripts as [`board::Observation`] rows (already past the
//! board's one ae-turn filter, so a marked or harness-wrapped turn never
//! reaches here as a row). Bodies stay RAW: the view routes every field
//! through the board's terminal renderer, the one place a control byte is made
//! inert.

use std::collections::BTreeMap;

use crate::board::{self, Role};
use crate::events::{Event, RoutingMember};
use crate::watchdog::{self, HUMAN_BRIDGE_ACTORS, WATCHDOG_ACTOR};
use crate::{reply, send, tracked};

/// The asker's withdrawal of its own request.
pub(super) const CANCEL: &str = "cancel";

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
    /// The human's console ask `id` to `to`; `uncertain` when its submit was
    /// not confirmed.
    Asked {
        to: String,
        id: String,
        uncertain: bool,
    },
    /// A console ask ae gave up delivering.
    NotDelivered { to: String, id: String },
    /// The human closed console request `id`.
    Closed { id: String },
    /// An ADMITTED reply to console request `id`: `follow_up` 0 is the answer,
    /// n the nth reply after it; `late` when the request was closed first;
    /// `gap` names why the body shown is the summary, not the stored whole.
    Answer {
        seat: String,
        id: String,
        follow_up: usize,
        late: bool,
        gap: Option<String>,
    },
    /// A reply to console request `id` that is NOT admitted as its answer.
    Unadmitted {
        from: String,
        id: String,
        why: String,
    },
}

/// What the console could read of an admitted answer's stored body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// The whole body, as the seat wrote it.
    Whole(String),
    /// The record names no body file: an older core wrote it.
    OldCore,
    /// The named file is not there.
    Missing,
    /// Refused, and why: a name or node the console will not read.
    Refused(String),
    /// Over [`reply::CONSOLE_BODY_CAP`].
    Oversized,
    /// Not UTF-8.
    NotUtf8,
}

/// Why an answer shows its summary instead of `body`; `None` for the whole.
fn body_gap(body: &Body) -> Option<String> {
    Some(match body {
        Body::Whole(_) => return None,
        Body::OldCore => "body missing (old core)".to_owned(),
        Body::Missing => "body missing".to_owned(),
        Body::Refused(why) => format!("body refused: {why}"),
        Body::Oversized => format!("body over {} bytes", reply::CONSOLE_BODY_CAP),
        Body::NotUtf8 => "body not UTF-8".to_owned(),
    })
}

/// One lane row: epoch micros (a journal record's second widened), what it
/// is, and its raw body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub micros: i64,
    pub kind: Kind,
    pub body: String,
    /// The position in the read of the journal record a console row came
    /// from: its identity across re-reads, whatever that record's stamp or
    /// body read says this time.
    pub record: Option<usize>,
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
        record: None,
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
    body: &dyn Fn(&Event) -> Body,
) -> Lane {
    let (mut items, other_say) = journal_items(session, seats, events);
    items.extend(standing_items(session, seats, events));
    let mut body_gaps = Vec::new();
    let console = console_items(events, body, &mut body_gaps);
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
    // The console's own thread keeps journal order, whatever its stamps say.
    let mut rest = std::mem::take(&mut items).into_iter().peekable();
    for row in console {
        items.extend(std::iter::from_fn(|| {
            rest.next_if(|item| item.micros <= row.micros)
        }));
        items.push(row);
    }
    items.extend(rest);
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
    coverage.extend(body_gaps);
    Lane { items, coverage }
}

/// The console's own thread: its asks and what became of them, and every
/// reply sent to it — the ANSWER only from the complete identity the ask
/// reached, with the stored whole body where it reads, else the summary and a
/// named gap. Follow-ups and lateness are by position, so a re-read of the
/// same journal draws the same rows.
fn console_items(
    events: &[Event],
    body: &dyn Fn(&Event) -> Body,
    gaps: &mut Vec<String>,
) -> Vec<Item> {
    // Per console request: its ask, whether a console cancel came before now,
    // and how many admitted replies came before now.
    let mut threads: BTreeMap<&str, (&Event, bool, usize)> = BTreeMap::new();
    let mut items = Vec::new();
    for (record, event) in events.iter().enumerate() {
        let key = event.reference.as_deref().unwrap_or("");
        let (id, to) = (key.to_owned(), event.target.clone().unwrap_or_default());
        let mut text = event.summary.clone().unwrap_or_default();
        let kind = if event.actor == tracked::CONSOLE_SINK {
            match event.action.as_str() {
                action if action == tracked::Kind::Ask.action() => {
                    if !key.is_empty() {
                        threads.insert(key, (event, false, 0));
                    }
                    let uncertain = text.starts_with(tracked::UNCONFIRMED_SUMMARY_PREFIX);
                    Kind::Asked { to, id, uncertain }
                }
                tracked::ABANDONED_ACTION => Kind::NotDelivered { to, id },
                CANCEL => {
                    if let Some(thread) = threads.get_mut(key) {
                        thread.1 = true;
                    }
                    Kind::Closed { id }
                }
                _ => continue,
            }
        } else if event.action == reply::ACTION && to == tracked::CONSOLE_SINK {
            let from = event.actor.clone();
            let admitted = match threads.get_mut(key) {
                None => Err("no console ask with this id in the journal".to_owned()),
                Some(thread) => admission(thread.0, event).map(|()| thread),
            };
            match admitted {
                Ok(thread) => {
                    let (late, follow_up) = (thread.1, thread.2);
                    thread.2 += 1;
                    let read = body(event);
                    let gap = body_gap(&read);
                    if let Body::Whole(whole) = read {
                        text = whole;
                    }
                    if let Some(gap) = &gap {
                        gaps.push(format!(
                            "reply {id} — {gap}; its 600-character summary is shown"
                        ));
                    }
                    let seat = from;
                    Kind::Answer {
                        seat,
                        id,
                        follow_up,
                        late,
                        gap,
                    }
                }
                Err(why) => Kind::Unadmitted { from, id, why },
            }
        } else {
            continue;
        };
        let record = Some(record);
        items.push(Item {
            record,
            ..item(kind, micros(event), &text)
        });
    }
    items
}

/// Whether `/close` may withdraw console request `id`: the console's own ask
/// in this journal, not answered by an admitted reply and not closed — read
/// from the same fold that draws the thread.
///
/// # Errors
///
/// Why not, by name.
pub fn may_close(events: &[Event], id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("a close needs a request id".to_owned());
    }
    let mut open = None;
    for item in console_items(events, &|_| Body::OldCore, &mut Vec::new()) {
        let still_open = open == Some(Ok(()));
        match item.kind {
            Kind::Asked { id: asked, .. } if asked == id => open = Some(Ok(())),
            Kind::Answer { id: answered, .. } if answered == id && still_open => {
                open = Some(Err(format!("{id} is already answered")));
            }
            Kind::Closed { id: closed } if closed == id && still_open => {
                open = Some(Err(format!("{id} is already closed")));
            }
            _ => {}
        }
    }
    open.unwrap_or_else(|| Err(format!("no console ask {id} in this journal")))
}

/// The ids of the console's asks still open in `session`'s journal, oldest
/// first. One forward pass: an ask with an id opens, and only three records close it —
/// an ADMITTED reply, the console's own cancel, and a retire
/// [`crate::session::retired`] judges. A stale, unproven or spawned reply is
/// not the answer, so its ask stays open.
#[must_use]
pub fn open_asks<'a>(events: &'a [Event], session: &str) -> Vec<&'a str> {
    let mut open: Vec<&Event> = Vec::new();
    for event in events {
        let key = event.reference.as_deref().filter(|key| !key.is_empty());
        let console = event.actor == tracked::CONSOLE_SINK;
        let answer =
            event.action == reply::ACTION && event.target.as_deref() == Some(tracked::CONSOLE_SINK);
        if event.action == "retire" {
            open.retain(|ask| !crate::session::retired(ask, event, session));
        } else if console && event.action == tracked::Kind::Ask.action() && key.is_some() {
            open.retain(|ask| ask.reference.as_deref() != key);
            open.push(event);
        } else if (console && event.action == CANCEL) || answer {
            let closes = |ask: &Event| !answer || admission(ask, event).is_ok();
            open.retain(|ask| ask.reference.as_deref() != key || !closes(ask));
        }
    }
    open.iter()
        .filter_map(|ask| ask.reference.as_deref())
        .collect()
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

/// Whether `reply` comes from the complete identity `ask` reached: no spawned
/// seat, no identity gap, and both five-tuples whole and equal. The error names
/// the first field that fails — `unproven` when a side lacks it, `stale` when
/// the sides differ.
fn admission(ask: &Event, reply: &Event) -> Result<(), String> {
    let spawned = |slot: &RoutingMember| {
        slot.value()
            .is_some_and(|slot| slot.starts_with("spawned."))
    };
    if spawned(&ask.target_slot) || spawned(&reply.actor_slot) {
        return Err("spawned seat".to_owned());
    }
    if let Some(gap) = &reply.identity_gap {
        return Err(format!("unproven identity ({gap})"));
    }
    let fields = [
        ("slot", ask.target_slot.value(), reply.actor_slot.value()),
        (
            "session",
            ask.target_session.value(),
            reply.actor_session.value(),
        ),
        (
            "server",
            ask.target_server.as_deref(),
            reply.caller_server.as_deref(),
        ),
        (
            "pane",
            ask.target_pane.as_deref(),
            reply.caller_pane.as_deref(),
        ),
        (
            "session uuid",
            ask.target_session_uuid.as_deref(),
            reply.caller_session_uuid.as_deref(),
        ),
    ];
    for (field, asked, answered) in fields {
        match (asked, answered) {
            (Some(asked), Some(answered)) if asked == answered => {}
            (Some(_), Some(_)) => return Err(format!("stale {field}")),
            _ => return Err(format!("unproven {field}")),
        }
    }
    Ok(())
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
    use super::{Body, Kind, Lane, Seat, fold};
    use crate::board::{Coverage, Observation, Role, Row};
    use crate::console::view::{Printed, tag};
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
        let lane = fold(S, &pair(), events, &observation, 0, &|_| Body::OldCore);
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
                    other => format!("{other:?}"),
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
        let lane = fold(S, &pair(), &events, &observation, 0, &|_| Body::OldCore);
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
        let lane: Lane = fold(S, &pair(), &events, &observation, 1, &|_| Body::OldCore);
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

    const ID: &str = "ae-20260930T060000Z-0000abcd";
    const UUID: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cc4321";

    /// A record about console request `ID`; `extra` is appended verbatim.
    fn about(ts: &str, actor: &str, action: &str, extra: &str) -> Event {
        let line =
            format!(r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}","ref":"{ID}"{extra}}}"#);
        Event::parse_line(&line).unwrap()
    }

    /// The console's ask to `lead`, whose identity the ask proved: `main`, `%1`.
    fn asked(summary: &str) -> Event {
        let target = format!(
            r#","target":"lead","target_slot":"main","target_session":"{S}","target_server":"/tmp/ae","target_pane":"%1","target_session_uuid":"{UUID}","summary":"{summary}""#
        );
        about(T0, "console:local", "ask", &target)
    }

    /// The caller fields of `slot` in pane `pane` of session `uuid`.
    fn caller(slot: &str, pane: &str, uuid: &str) -> String {
        format!(
            r#","actor_slot":"{slot}","actor_session":"{S}","caller_server":"/tmp/ae","caller_pane":"{pane}","caller_session_uuid":"{uuid}""#
        )
    }

    /// `lead`'s reply to the console, stamped with `fields`.
    fn answered(ts: &str, fields: &str, summary: &str) -> Event {
        let extra = format!(r#","target":"console:local"{fields},"summary":"{summary}""#);
        about(ts, "lead", "reply", &extra)
    }

    fn whole(event: &Event) -> Body {
        Body::Whole(format!("WHOLE {}", event.summary.as_deref().unwrap_or("")))
    }

    /// The lane as the view tags it, and its coverage.
    fn thread(events: &[Event], body: &dyn Fn(&Event) -> Body) -> (Vec<String>, Vec<String>) {
        let lane = fold(S, &pair(), events, &Observation::default(), 0, body);
        let rows = lane
            .items
            .iter()
            .map(|item| format!("{}: {}", tag(&item.kind), item.body));
        (rows.collect(), lane.coverage)
    }

    fn refused(why: &str) -> String {
        format!("lead reply to {ID} · not admitted: {why} · preview (600-char summary): x")
    }

    #[test]
    fn an_admitted_answer_is_the_whole_body_and_each_later_admitted_reply_a_follow_up() {
        let me = caller("main", "%1", UUID);
        let events = [
            asked("q"),
            answered(T1, &caller("main", "%2", UUID), "x"),
            answered(T1, &me, "a1"),
            answered(T1, &me, "a1"),
            answered(T2, &me, "a2"),
        ];
        assert_eq!(
            thread(&events, &whole).0,
            [
                format!("you → lead · {ID}: q"),
                refused("stale pane"),
                format!("lead answers {ID}: WHOLE a1"),
                format!("lead answers {ID} · follow-up 1: WHOLE a1"),
                format!("lead answers {ID} · follow-up 2: WHOLE a2"),
            ],
            "an unadmitted reply takes no place in the count"
        );
    }

    #[test]
    fn a_reply_is_admitted_only_from_the_whole_identity_the_ask_reached() {
        let (me, other) = (
            caller("main", "%1", UUID),
            "0199c0de-1234-4890-abcd-ef0123456789",
        );
        let (bare, gap) = (
            format!(r#","actor_slot":"main","actor_session":"{S}""#),
            format!(r#"{me},"identity_gap":"vacant""#),
        );
        let cases = [
            (caller("main", "%1", other), "stale session uuid"),
            (caller("worker.0", "%1", UUID), "stale slot"),
            (me.replace("/tmp/ae", "/tmp/other"), "stale server"),
            (me.replace(S, "elsewhere"), "stale session"),
            (caller("spawned.0", "%1", UUID), "spawned seat"),
            (bare, "unproven server"),
            (gap, "unproven identity (vacant)"),
        ];
        for (fields, why) in cases {
            let events = [asked("q"), answered(T1, &fields, "x")];
            assert_eq!(thread(&events, &whole).0[1], refused(why), "{why}");
        }
        let partial = r#","target":"lead","target_slot":"main","target_session":"aedev","target_server":"/tmp/ae""#;
        let unproven_ask = about(T0, "console:local", "ask", partial);
        let events = [unproven_ask, answered(T1, &me, "x")];
        assert_eq!(thread(&events, &whole).0[1], refused("unproven pane"));
        // No request id on either side is no join, however whole both identities are.
        let (mut bare_ask, mut bare_reply) = (asked("q"), answered(T1, &me, "x"));
        (bare_ask.reference, bare_reply.reference) = (None, None);
        let rows = thread(&[bare_ask, bare_reply], &whole).0;
        assert!(rows[1].contains("not admitted: no console ask"), "{rows:?}");
        let orphan = [answered(T1, &me, "x")];
        assert_eq!(
            thread(&orphan, &whole).0,
            [refused("no console ask with this id in the journal")]
        );
    }

    #[test]
    fn an_uncertain_ask_an_abandoned_one_a_close_and_a_late_answer_each_show() {
        let me = caller("main", "%1", UUID);
        let (lost, closed) = (
            r#","target":"lead","summary":"refused: busy""#,
            r#","summary":"by you""#,
        );
        let events = [
            asked("[unconfirmed] q"),
            about(T0, "console:local", "delivery-abandoned", lost),
            answered(T1, &me, "before"),
            about(T1, "lead", "cancel", r#","summary":"not the asker""#),
            about(T2, "console:local", "cancel", closed),
            answered(T2, &me, "after"),
        ];
        assert_eq!(
            thread(&events, &whole).0,
            [
                format!("you → lead · {ID} · uncertain: check the lead pane: [unconfirmed] q"),
                format!("you → lead · {ID} · not delivered: refused: busy"),
                format!("lead answers {ID}: WHOLE before"),
                format!("you closed {ID}: by you"),
                format!("lead answers {ID} · follow-up 1 · late (closed): WHOLE after"),
            ]
        );
    }

    #[test]
    fn a_close_needs_the_consoles_own_ask_unanswered_and_unclosed() {
        let me = caller("main", "%1", UUID);
        let stale = caller("main", "%2", UUID);
        let lead_asks = about(T0, "lead", "ask", r#","target":"colead""#);
        let cancel = |actor| about(T1, actor, "cancel", "");
        let mut elsewhere = cancel("console:local");
        elsewhere.reference = Some("ae-other".to_owned());
        let (none, answered_, closed) = (
            format!("no console ask {ID} in this journal"),
            format!("{ID} is already answered"),
            format!("{ID} is already closed"),
        );
        let cases = [
            (vec![asked("q")], Ok(())),
            (vec![], Err(none.clone())),
            (vec![lead_asks], Err(none)),
            (vec![asked("q"), answered(T1, &stale, "a")], Ok(())),
            (vec![asked("q"), cancel("lead")], Ok(())),
            (vec![asked("q"), elsewhere], Ok(())),
            (vec![asked("q"), answered(T1, &me, "a")], Err(answered_)),
            (
                vec![asked("q"), cancel("console:local")],
                Err(closed.clone()),
            ),
            (
                vec![asked("q"), cancel("console:local"), answered(T2, &me, "a")],
                Err(closed),
            ),
            (
                vec![asked("q"), answered(T1, &me, "a"), asked("again")],
                Ok(()),
            ),
        ];
        for (events, verdict) in cases {
            assert_eq!(super::may_close(&events, ID), verdict, "{events:?}");
        }
        let other = super::may_close(&[asked("q")], "ae-other");
        let none = Err("no console ask ae-other in this journal".to_owned());
        assert_eq!(other, none);
        let refless = r#"{"ts":"2026-09-30T06:00:00Z","actor":"console:local","action":"ask"}"#;
        let refless = Event::parse_line(refless).unwrap();
        let empty = super::may_close(&[refless], "");
        assert_eq!(empty, Err("a close needs a request id".to_owned()));
    }

    #[test]
    fn an_ask_stays_open_until_its_answer_its_close_or_its_seats_retire() {
        let (me, stale) = (caller("main", "%1", UUID), caller("main", "%2", UUID));
        let unproven = format!(r#"{me},"identity_gap":"vacant""#);
        let to_colead = format!(r#","target":"colead"{me},"summary":"a""#);
        let not_to_me = about(T1, "lead", "reply", &to_colead);
        let spawned = caller("spawned.0", "%1", UUID);
        let retire =
            |slot: &str| about(T1, "lead", "retire", &format!(r#","target_slot":"{slot}""#));
        let mut refless = asked("q");
        refless.reference = Some(String::new());
        let cases = [
            (vec![asked("q")], S, 1),
            (vec![asked("[unconfirmed] q")], S, 1),
            (vec![refless], S, 0),
            (
                vec![about(T0, "console:local", "delivery-abandoned", "")],
                S,
                0,
            ),
            (vec![asked("q"), answered(T1, &me, "a")], S, 0),
            (vec![answered(T0, &me, "a"), asked("q")], S, 1),
            (vec![asked("q"), answered(T1, &stale, "a")], S, 1),
            (vec![asked("q"), not_to_me], S, 1),
            (vec![asked("q"), answered(T1, &unproven, "a")], S, 1),
            (vec![asked("q"), answered(T1, &spawned, "a")], S, 1),
            (
                vec![asked("q"), about(T1, "console:local", "cancel", "")],
                S,
                0,
            ),
            (vec![asked("q"), about(T1, "lead", "cancel", "")], S, 1),
            (vec![asked("q"), retire("main")], S, 0),
            (vec![asked("q"), retire("main")], "elsewhere", 1),
            (vec![asked("q"), retire("worker.0")], S, 1),
            (
                vec![asked("q"), answered(T1, &me, "a"), asked("again")],
                S,
                1,
            ),
        ];
        for (events, session, want) in cases {
            let open = super::open_asks(&events, session);
            assert_eq!(open.len(), want, "{session} {events:?}");
        }
        let mut other = asked("other");
        other.reference = Some("ae-other".to_owned());
        let events = [asked("q"), other, answered(T1, &me, "a")];
        assert_eq!(super::open_asks(&events, S), ["ae-other"]);
    }

    #[test]
    fn an_answer_whose_body_cannot_be_read_names_why_and_keeps_its_summary() {
        let cases = [
            (Body::OldCore, "body missing (old core)"),
            (Body::Missing, "body missing"),
            (Body::Refused("x".to_owned()), "body refused: x"),
            (Body::Oversized, "body over 65536 bytes"),
            (Body::NotUtf8, "body not UTF-8"),
        ];
        let events = [
            asked("q"),
            answered(T1, &caller("main", "%1", UUID), "short"),
        ];
        for (body, gap) in cases {
            let (rows, coverage) = thread(&events, &|_| body.clone());
            let row = format!("lead answers {ID} · {gap} · preview (600-char summary): short");
            let named = format!("reply {ID} — {gap}; its 600-character summary is shown");
            assert_eq!((rows[1].as_str(), coverage), (row.as_str(), vec![named]));
        }
    }

    /// A console row is its record's place in the read: two identical records
    /// are two rows, and a re-read prints neither again, whatever its stamp or
    /// body read says.
    #[test]
    fn a_console_row_is_its_record_printed_once_however_its_body_reads_later() {
        let me = caller("main", "%1", UUID);
        let events = |ts| [asked("q"), answered(ts, &me, "a"), answered(ts, &me, "a")];
        let observed = Observation::default();
        let (first, later) = (events(T1), events(T2));
        let mut printed = Printed::default();
        let lane = fold(S, &pair(), &first, &observed, 0, &|_| Body::Missing);
        let shown = printed.step(&lane, 0, true);
        assert_eq!(shown.matches(" answers ").count(), 2, "{shown}");
        let lane = fold(S, &pair(), &later, &observed, 0, &whole);
        let shown = printed.step(&lane, 0, true);
        assert_eq!(shown, "", "the same records, restamped and read whole");
    }

    /// The console's thread keeps journal order: a reply stamped earlier than
    /// the one appended before it still follows it.
    #[test]
    fn the_console_thread_keeps_journal_order_whatever_its_stamps_say() {
        let me = caller("main", "%1", UUID);
        let events = [asked("q"), answered(T2, &me, "1"), answered(T1, &me, "2")];
        let rows = vec![row("lead", Role::Human, 0, MICROS_T1 + 30_000_000, "typed")];
        let observed = Observation {
            rows,
            ..Observation::default()
        };
        let lane = fold(S, &pair(), &events, &observed, 0, &whole);
        let shown: Vec<String> = lane.items.iter().map(|item| tag(&item.kind)).collect();
        let answer = format!("lead answers {ID}");
        let expected = [
            format!("you → lead · {ID}"),
            "lead pane (transcript)".to_owned(),
            answer.clone(),
            format!("{answer} · follow-up 1"),
        ];
        assert_eq!(shown, expected);
    }
}
