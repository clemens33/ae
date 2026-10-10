//! `_ar_request_states` — the digest's own request-status pass.
//!
//! This is NOT [`crate::requests`]. Two differences matter and both are
//! deliberate:
//!
//! * the digest keeps only ONE opening per ref — the newest `ask`/`review`,
//!   found by scanning the container newest-first and taking the first seen;
//! * cancel authorization here is the request's own SENDER by exact actor
//!   bytes, or slot+session when the opening and the cancel both carry a slot —
//!   not the slotless-sender rule that lives in [`crate::requests`]. For a
//!   `compact --digest-only` withdrawal the two agree (the cancel's actor
//!   equals the opening's sender bytes), so the digest never had the view's
//!   pending-forever bug, and the preview stays byte-identical.
//!
//! A reply closes the request only on the FULL mirror (actor is the request's
//! target, target is the request's sender) — by slot+session when both the
//! request's target and the reply's actor carry a slot, else by name. A valid
//! withdrawal is terminal and wins over any reply. Only `pending` rows reach
//! the digest.

use crate::event_text::{extract, reversed};
use crate::requests::{Key, same_session, session_ids};

/// One request row as the digest consumes it.
pub(super) struct RequestRow {
    pub status: String,
    pub kind: String,
    pub reference: String,
    pub from: String,
    pub to: String,
    pub ts: String,
    pub body_file: String,
    pub summary: String,
}

struct Opening {
    kind: String,
    sender: String,
    target: String,
    actor_slot: String,
    target_slot: String,
    actor_session: String,
    target_session: String,
    actor_session_id: Key,
    target_session_id: Key,
    summary: String,
    ts: String,
    body_file: String,
}

/// A `reply` candidate: the fields the mirror test needs.
struct Reply {
    actor: String,
    target: String,
    actor_slot: String,
    target_slot: String,
    actor_session: String,
    target_session: String,
    actor_session_id: Key,
    target_session_id: Key,
    summary: String,
}

/// A `cancel` candidate.
struct Cancel {
    actor: String,
    actor_slot: String,
    actor_session: String,
    actor_session_id: Key,
    summary: String,
}

fn field(line: &[u8], key: &str) -> String {
    String::from_utf8_lossy(&extract(line, key)).into_owned()
}

/// The FULL mirror, by slot+session when both the request's target and the
/// reply's actor carry a slot, else by name.
fn reply_closes(opening: &Opening, reply: &Reply) -> bool {
    if !opening.target_slot.is_empty() && !reply.actor_slot.is_empty() {
        reply.actor_slot == opening.target_slot
            && same_session(
                (&reply.actor_session, &reply.actor_session_id),
                (&opening.target_session, &opening.target_session_id),
            )
            && reply.target_slot == opening.actor_slot
            && same_session(
                (&reply.target_session, &reply.target_session_id),
                (&opening.actor_session, &opening.actor_session_id),
            )
    } else {
        reply.actor == opening.target && reply.target == opening.sender
    }
}

/// Interim withdrawal authorization: the request's own sender, by slot+session
/// when both carry a slot, else by exact non-empty actor bytes.
fn cancel_closes(opening: &Opening, cancel: &Cancel) -> bool {
    if !opening.actor_slot.is_empty() && !cancel.actor_slot.is_empty() {
        cancel.actor_slot == opening.actor_slot
            && same_session(
                (&cancel.actor_session, &cancel.actor_session_id),
                (&opening.actor_session, &opening.actor_session_id),
            )
    } else {
        !cancel.actor.is_empty() && cancel.actor == opening.sender
    }
}

pub(super) fn request_states(event_bytes: &[u8]) -> Vec<RequestRow> {
    // Newest-first.
    let reversed = reversed(event_bytes);
    let mut order: Vec<String> = Vec::new();
    let mut openings: std::collections::HashMap<String, (usize, Opening)> =
        std::collections::HashMap::new();
    // Candidates newest-first, in scan order (already newest-first here).
    let mut replies: std::collections::HashMap<String, Vec<(usize, Reply)>> =
        std::collections::HashMap::new();
    let mut cancels: std::collections::HashMap<String, Vec<(usize, Cancel)>> =
        std::collections::HashMap::new();
    // The scan is newest-first, so a SMALLER ordinal is a LATER record.
    let mut scan = 0_usize;

    for line in crate::event_text::read_lines(&reversed) {
        if line.first() != Some(&b'{') {
            continue;
        }
        scan += 1;
        let reference = field(line, "ref");
        if reference.is_empty() {
            continue;
        }
        match field(line, "action").as_str() {
            "ask" | "review" => {
                // First seen wins = newest (the scan is newest-first).
                if openings.contains_key(&reference) {
                    continue;
                }
                let summary = field(line, "summary").replace('\n', " ");
                let [actor_session_id, target_session_id] = session_ids(line);
                openings.insert(
                    reference.clone(),
                    (
                        scan,
                        Opening {
                            kind: field(line, "action"),
                            sender: field(line, "actor"),
                            target: field(line, "target"),
                            actor_slot: field(line, "actor_slot"),
                            target_slot: field(line, "target_slot"),
                            actor_session: field(line, "actor_session"),
                            target_session: field(line, "target_session"),
                            actor_session_id,
                            target_session_id,
                            summary,
                            ts: field(line, "ts"),
                            body_file: field(line, "body_file"),
                        },
                    ),
                );
                order.push(reference);
            }
            "reply" => {
                let [actor_session_id, target_session_id] = session_ids(line);
                replies.entry(reference).or_default().push((
                    scan,
                    Reply {
                        actor: field(line, "actor"),
                        target: field(line, "target"),
                        actor_slot: field(line, "actor_slot"),
                        target_slot: field(line, "target_slot"),
                        actor_session: field(line, "actor_session"),
                        target_session: field(line, "target_session"),
                        actor_session_id,
                        target_session_id,
                        summary: field(line, "summary"),
                    },
                ));
            }
            "cancel" => {
                let [actor_session_id, _] = session_ids(line);
                cancels.entry(reference).or_default().push((
                    scan,
                    Cancel {
                        actor: field(line, "actor"),
                        actor_slot: field(line, "actor_slot"),
                        actor_session: field(line, "actor_session"),
                        actor_session_id,
                        summary: field(line, "summary"),
                    },
                ));
            }
            _ => {}
        }
    }

    // The output loop iterates the openings OLDEST-first, reversing the
    // newest-first `order`.
    order
        .iter()
        .rev()
        .map(|reference| {
            let (opened_at, opening) = &openings[reference];
            row_for(reference, *opened_at, opening, &replies, &cancels)
        })
        .collect()
}

/// One row: the opening, plus whichever terminal event ended it.
fn row_for(
    reference: &str,
    opened_at: usize,
    opening: &Opening,
    replies: &std::collections::HashMap<String, Vec<(usize, Reply)>>,
    cancels: &std::collections::HashMap<String, Vec<(usize, Cancel)>>,
) -> RequestRow {
    // A terminal event ends only an opening it FOLLOWS. The scan is
    // newest-first, so "after the opening" is a SMALLER ordinal — without this
    // an old withdrawal reaches forward and closes a request that was asked
    // again on the same ref. Candidates are newest-first, so the first that
    // passes both tests is the newest that counts.
    let rep = replies
        .get(reference)
        .into_iter()
        .flatten()
        .filter(|(at, _)| *at < opened_at)
        .find(|(_, candidate)| reply_closes(opening, candidate))
        .map(|(_, candidate)| candidate.summary.clone());
    let can = cancels
        .get(reference)
        .into_iter()
        .flatten()
        .filter(|(at, _)| *at < opened_at)
        .find(|(_, candidate)| cancel_closes(opening, candidate))
        .map(|(_, candidate)| candidate.summary.clone());

    // A valid withdrawal wins over any reply, however late.
    let (status, summary) = match (can, rep) {
        (Some(text), _) => ("cancelled", text),
        (None, Some(text)) => ("replied", text),
        (None, None) => ("pending", opening.summary.clone()),
    };
    RequestRow {
        status: status.to_owned(),
        kind: opening.kind.clone(),
        reference: reference.to_owned(),
        from: opening.sender.clone(),
        to: opening.target.clone(),
        ts: opening.ts.clone(),
        body_file: opening.body_file.clone(),
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::request_states;
    use crate::events::{Cursor, Drain, Event};
    use crate::requests::{Status, states};
    use crate::session::SessionRead;

    /// One ask, then a withdrawal of it by the same sender.
    const WITHDRAWN: &str = concat!(
        r#"{"ts":"2026-05-29T09:00:00Z","actor":"cl:lead","action":"ask","#,
        r#""target":"cl:hand","ref":"ae-1","summary":"q"}"#,
        "\n",
        r#"{"ts":"2026-05-29T09:05:00Z","actor":"cl:lead","action":"cancel","#,
        r#""target":"cl:hand","ref":"ae-1","summary":"withdrawn"}"#,
        "\n",
    );

    /// One ask, then the target's reply to the asker — the full mirror every
    /// reader agrees closes a request.
    const ANSWERED: &str = concat!(
        r#"{"ts":"2026-05-29T09:00:00Z","actor":"cl:lead","action":"ask","#,
        r#""target":"cl:hand","ref":"ae-2","summary":"q"}"#,
        "\n",
        r#"{"ts":"2026-05-29T09:05:00Z","actor":"cl:hand","action":"reply","#,
        r#""target":"cl:lead","ref":"ae-2","summary":"a"}"#,
        "\n",
    );

    /// THE SHAPE `compact --digest-only` ACTUALLY WRITES, which is why it is in
    /// this corpus: `tracked::run` records the compact sender's session but its
    /// slot is empty, so the opening is half-routed; the withdrawal goes out
    /// through the plain event writer with no routing member at all. A reader
    /// that only compares routing keys leaves this request open forever.
    const COMPACT_WITHDRAWN: &str = concat!(
        r#"{"ts":"2026-05-29T09:00:00Z","actor":"ae:compact:0199c0de","action":"ask","#,
        r#""target":"cl:lead","ref":"ae-4","actor_session":"demo","summary":"hand over"}"#,
        "\n",
        r#"{"ts":"2026-05-29T09:05:00Z","actor":"ae:compact:0199c0de","action":"cancel","#,
        r#""ref":"ae-4","summary":"withdrawn: --digest-only"}"#,
        "\n",
    );

    /// A withdrawal, then a fresh ask on the same ref: the new opening is open.
    const REOPENED: &str = concat!(
        r#"{"ts":"2026-05-29T09:00:00Z","actor":"cl:lead","action":"ask","#,
        r#""target":"cl:hand","ref":"ae-5","summary":"q"}"#,
        "\n",
        r#"{"ts":"2026-05-29T09:05:00Z","actor":"cl:lead","action":"cancel","#,
        r#""target":"cl:hand","ref":"ae-5","summary":"withdrawn"}"#,
        "\n",
        r#"{"ts":"2026-05-29T09:10:00Z","actor":"cl:lead","action":"ask","#,
        r#""target":"cl:hand","ref":"ae-5","summary":"asked again"}"#,
        "\n",
    );

    /// A withdrawal, then a straggler reply: the withdrawal already ended it.
    const WITHDRAWN_THEN_ANSWERED: &str = concat!(
        r#"{"ts":"2026-05-29T09:00:00Z","actor":"cl:lead","action":"ask","#,
        r#""target":"cl:hand","ref":"ae-6","summary":"q"}"#,
        "\n",
        r#"{"ts":"2026-05-29T09:05:00Z","actor":"cl:lead","action":"cancel","#,
        r#""target":"cl:hand","ref":"ae-6","summary":"withdrawn"}"#,
        "\n",
        r#"{"ts":"2026-05-29T09:10:00Z","actor":"cl:hand","action":"reply","#,
        r#""target":"cl:lead","ref":"ae-6","summary":"too late"}"#,
        "\n",
    );

    /// One ask nobody touched.
    const OPEN: &str = concat!(
        r#"{"ts":"2026-05-29T09:00:00Z","actor":"cl:lead","action":"ask","#,
        r#""target":"cl:hand","ref":"ae-3","summary":"q"}"#,
        "\n",
    );

    /// What [`SessionRead`] holds open, which is what feeds the `unanswered`
    /// attention marker.
    fn session_pending(container: &str) -> Vec<String> {
        let events: Vec<Event> = container
            .lines()
            .map(|line| Event::parse_line(line).expect("a fixture event"))
            .collect();
        SessionRead::from_drain(&Drain {
            events,
            cursor: Cursor::default(),
            skipped: Vec::new(),
            drained: true,
        })
        .pending
        .iter()
        .map(|request| request.id.clone())
        .collect()
    }

    /// The digest's own reader, as its callers consume it.
    fn digest_pending(container: &str) -> Vec<String> {
        request_states(container.as_bytes())
            .into_iter()
            .filter(|row| row.status == "pending")
            .map(|row| row.reference)
            .collect()
    }

    /// The view's reader, reduced to the same shape.
    fn view_pending(container: &str) -> Vec<String> {
        states(container.as_bytes())
            .into_iter()
            .filter(|request| request.status == Status::Pending)
            .map(|request| String::from_utf8_lossy(&request.id).into_owned())
            .collect()
    }

    // THE THREE READERS ON ONE CORPUS. This pin lives beside the digest's
    // reader because this file is where the deliberate differences are already
    // written down; what it adds is the MEASURED answer of all three, so a
    // later unification is a diff against a fact rather than against a memory.
    #[test]
    fn an_open_request_is_open_to_every_reader() {
        assert_eq!(view_pending(OPEN), ["ae-3"]);
        assert_eq!(digest_pending(OPEN), ["ae-3"]);
        assert_eq!(session_pending(OPEN), ["ae-3"]);
    }

    #[test]
    fn a_full_mirror_reply_closes_the_request_for_every_reader() {
        assert!(view_pending(ANSWERED).is_empty());
        assert!(digest_pending(ANSWERED).is_empty());
        assert!(session_pending(ANSWERED).is_empty());
    }

    #[test]
    fn a_withdrawal_closes_the_request_for_every_reader() {
        assert!(
            view_pending(WITHDRAWN).is_empty(),
            "the view treats a valid withdrawal as terminal"
        );
        assert!(
            digest_pending(WITHDRAWN).is_empty(),
            "so does the digest, on its own cancel-authorization policy"
        );
        // The reader behind `SessionRead::unanswered`: a withdrawn request is
        // not one anybody is waiting on, so it contributes no attention.
        assert!(session_pending(WITHDRAWN).is_empty());
    }

    #[test]
    fn the_withdrawal_compact_actually_writes_closes_it_for_every_reader() {
        assert!(view_pending(COMPACT_WITHDRAWN).is_empty());
        assert!(digest_pending(COMPACT_WITHDRAWN).is_empty());
        assert!(session_pending(COMPACT_WITHDRAWN).is_empty());
    }

    #[test]
    fn a_fresh_ask_after_a_withdrawal_is_open_again_to_every_reader() {
        // A terminal event ends only an opening it FOLLOWS, in all three
        // readers: re-asking a withdrawn ref opens it again, and the earlier
        // withdrawal does not reach forward to close it.
        assert_eq!(view_pending(REOPENED), ["ae-5"]);
        assert_eq!(digest_pending(REOPENED), ["ae-5"]);
        assert_eq!(session_pending(REOPENED), ["ae-5"]);
    }

    #[test]
    fn a_reply_after_a_withdrawal_does_not_reopen_it_for_any_reader() {
        assert!(view_pending(WITHDRAWN_THEN_ANSWERED).is_empty());
        assert!(digest_pending(WITHDRAWN_THEN_ANSWERED).is_empty());
        assert!(session_pending(WITHDRAWN_THEN_ANSWERED).is_empty());
    }

    #[test]
    fn a_session_id_judges_a_reply_the_same_for_every_reader() {
        let id = "0199c0de-1111-4890-abcd-ef0123456789";
        let other = "0199c0de-2222-4890-abcd-ef0123456789";
        let pin = |key: &str, value: &str| format!(r#","{key}":"{value}""#);
        for (asked, replied, open) in [
            (
                pin("target_session_id", id),
                pin("actor_session_id", id),
                false,
            ),
            (pin("target_session_id", id), String::new(), false),
            (
                pin("target_session_id", id),
                pin("actor_session_id", other),
                true,
            ),
            (pin("target_session_id", "bogus"), String::new(), true),
            (pin("target_session_id", ""), String::new(), true),
            (String::new(), pin("actor_session_id", "bogus"), true),
            (
                pin("target_session_id", id),
                pin("actor_session_id", ""),
                true,
            ),
            // What only a JSON reader sees: a wrong type, a duplicate, a space.
            (
                pin("target_session_id", id),
                r#","actor_session_id":7"#.to_owned(),
                true,
            ),
            (
                pin("target_session_id", id),
                pin("actor_session_id", id) + &pin("actor_session_id", other),
                true,
            ),
            (
                pin("target_session_id", id),
                format!(r#","actor_session_id": "{other}""#),
                true,
            ),
            // The same two, with the key spelled through a JSON escape.
            (
                pin("target_session_id", id),
                r#","actor_session_\u0069d":7"#.to_owned(),
                true,
            ),
            (
                pin("target_session_id", id),
                format!(r#","actor_session_\u0069d":"{id}","actor_session_\u0069d":"{other}""#),
                true,
            ),
        ] {
            let corpus = format!(
                "{}{asked}{}\n{}{replied}{}\n",
                r#"{"ts":"2026-05-29T09:00:00Z","actor":"lead","action":"ask","target":"w","ref":"ae-9","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s""#,
                r#","summary":"q"}"#,
                r#"{"ts":"2026-05-29T09:05:00Z","actor":"w","action":"reply","target":"lead","ref":"ae-9","actor_slot":"worker.0","actor_session":"s","target_slot":"main","target_session":"s""#,
                r#","summary":"a"}"#,
            );
            let want: &[&str] = if open { &["ae-9"] } else { &[] };
            // The strict ledger drops a line its parser refuses, as a drain does.
            let events = corpus
                .lines()
                .filter_map(|line| Event::parse_line(line).ok());
            let ledger = SessionRead::from_drain(&Drain {
                events: events.collect(),
                cursor: Cursor::default(),
                skipped: Vec::new(),
                drained: true,
            });
            let strict: Vec<&str> = ledger.pending.iter().map(|r| r.id.as_str()).collect();
            assert_eq!(strict, want, "{corpus}");
            assert_eq!(view_pending(&corpus), want, "{corpus}");
            assert_eq!(digest_pending(&corpus), want, "{corpus}");
        }
    }

    /// `cancel_closes`' doc: only the request's own sender withdraws it — by
    /// slot+session when both carry a slot (the session judged by its stable
    /// id, so across a rename), else by exact non-empty actor bytes.
    #[test]
    fn only_the_senders_own_seat_withdraws_a_slotted_request() {
        const U: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cc4321";
        const V: &str = "6fa459ea-ee8a-3ca4-894e-db77e160355e";
        let ask = |reference: &str| {
            format!(
                r#"{{"ts":"2026-05-29T09:00:00Z","actor":"cl:lead","action":"ask","target":"cl:hand","ref":"{reference}","actor_slot":"main","actor_session":"s","actor_session_id":"{U}","target_slot":"worker.0","target_session":"s","target_session_id":"{U}","summary":"q"}}"#
            )
        };
        let cancel = |reference: &str, slot: &str, session: &str, id: &str| {
            let routed = if slot.is_empty() {
                String::new()
            } else {
                format!(
                    r#""actor_slot":"{slot}","actor_session":"{session}","actor_session_id":"{id}","#
                )
            };
            format!(
                r#"{{"ts":"2026-05-29T09:05:00Z","actor":"cl:lead","action":"cancel","ref":"{reference}",{routed}"summary":"withdrawn"}}"#
            )
        };
        let corpus = [
            ask("ae-renamed"),
            cancel("ae-renamed", "main", "new", U),
            ask("ae-other-slot"),
            cancel("ae-other-slot", "worker.0", "s", U),
            ask("ae-other-id"),
            cancel("ae-other-id", "main", "s", V),
            ask("ae-keyless"),
            cancel("ae-keyless", "", "", ""),
        ];
        let mut pending = digest_pending(&format!("{}\n", corpus.join("\n")));
        pending.sort();
        assert_eq!(pending, ["ae-other-id", "ae-other-slot"]);
    }
}
