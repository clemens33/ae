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
    summary: String,
}

/// A `cancel` candidate.
struct Cancel {
    actor: String,
    actor_slot: String,
    actor_session: String,
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
            && reply.actor_session == opening.target_session
            && reply.target_slot == opening.actor_slot
            && reply.target_session == opening.actor_session
    } else {
        reply.actor == opening.target && reply.target == opening.sender
    }
}

/// Interim withdrawal authorization: the request's own sender, by slot+session
/// when both carry a slot, else by exact non-empty actor bytes.
fn cancel_closes(opening: &Opening, cancel: &Cancel) -> bool {
    if !opening.actor_slot.is_empty() && !cancel.actor_slot.is_empty() {
        cancel.actor_slot == opening.actor_slot && cancel.actor_session == opening.actor_session
    } else {
        !cancel.actor.is_empty() && cancel.actor == opening.sender
    }
}

pub(super) fn request_states(event_bytes: &[u8]) -> Vec<RequestRow> {
    // Newest-first.
    let reversed = reversed(event_bytes);
    let mut order: Vec<String> = Vec::new();
    let mut openings: std::collections::HashMap<String, Opening> = std::collections::HashMap::new();
    // Candidates newest-first, in scan order (already newest-first here).
    let mut replies: std::collections::HashMap<String, Vec<Reply>> =
        std::collections::HashMap::new();
    let mut cancels: std::collections::HashMap<String, Vec<Cancel>> =
        std::collections::HashMap::new();

    for line in crate::event_text::read_lines(&reversed) {
        if line.first() != Some(&b'{') {
            continue;
        }
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
                openings.insert(
                    reference.clone(),
                    Opening {
                        kind: field(line, "action"),
                        sender: field(line, "actor"),
                        target: field(line, "target"),
                        actor_slot: field(line, "actor_slot"),
                        target_slot: field(line, "target_slot"),
                        actor_session: field(line, "actor_session"),
                        target_session: field(line, "target_session"),
                        summary,
                        ts: field(line, "ts"),
                        body_file: field(line, "body_file"),
                    },
                );
                order.push(reference);
            }
            "reply" => {
                replies.entry(reference).or_default().push(Reply {
                    actor: field(line, "actor"),
                    target: field(line, "target"),
                    actor_slot: field(line, "actor_slot"),
                    target_slot: field(line, "target_slot"),
                    actor_session: field(line, "actor_session"),
                    target_session: field(line, "target_session"),
                    summary: field(line, "summary"),
                });
            }
            "cancel" => {
                cancels.entry(reference).or_default().push(Cancel {
                    actor: field(line, "actor"),
                    actor_slot: field(line, "actor_slot"),
                    actor_session: field(line, "actor_session"),
                    summary: field(line, "summary"),
                });
            }
            _ => {}
        }
    }

    // The output loop iterates the openings OLDEST-first, reversing the
    // newest-first `order`.
    let mut rows = Vec::new();
    for reference in order.iter().rev() {
        let opening = &openings[reference];
        let mut status = "pending";
        let mut summary = opening.summary.clone();

        // Newest valid reply, then newest valid withdrawal (candidates are
        // newest-first, so the first that passes is the newest that counts).
        let rep = replies
            .get(reference)
            .into_iter()
            .flatten()
            .find(|c| reply_closes(opening, c))
            .map(|c| c.summary.clone());
        let can = cancels
            .get(reference)
            .into_iter()
            .flatten()
            .find(|c| cancel_closes(opening, c))
            .map(|c| c.summary.clone());

        if let Some(text) = can {
            status = "cancelled";
            summary = text;
        } else if let Some(text) = rep {
            status = "replied";
            summary = text;
        }

        rows.push(RequestRow {
            status: status.to_owned(),
            kind: opening.kind.clone(),
            reference: reference.clone(),
            from: opening.sender.clone(),
            to: opening.target.clone(),
            ts: opening.ts.clone(),
            body_file: opening.body_file.clone(),
            summary,
        });
    }
    rows
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
    fn a_reply_after_a_withdrawal_does_not_reopen_it_for_any_reader() {
        assert!(view_pending(WITHDRAWN_THEN_ANSWERED).is_empty());
        assert!(digest_pending(WITHDRAWN_THEN_ANSWERED).is_empty());
        assert!(session_pending(WITHDRAWN_THEN_ANSWERED).is_empty());
    }
}
