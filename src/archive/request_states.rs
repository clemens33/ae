//! `_ar_request_states` — the digest's own request-status pass.
//!
//! This is a FAITHFUL port of the frozen bash reader, NOT [`crate::requests`].
//! Two differences matter and both are deliberate:
//!
//! * the digest keeps only ONE opening per ref — the newest `ask`/`review`,
//!   found by scanning the container newest-first and taking the first seen;
//! * cancel authorization is the frozen `_ar_request_states` policy (the
//!   request's own SENDER by exact actor bytes, or slot+session when the
//!   opening and the cancel both carry a slot) — the INTERIM policy, not the
//!   later slotless-sender ruling that lives in [`crate::requests`]. For a
//!   `compact --digest-only` withdrawal the two agree (the cancel's actor
//!   equals the opening's sender bytes), so the digest never had the view's
//!   pending-forever bug; matching the frozen reader here is what keeps the
//!   preview byte-identical.
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
/// reply's actor carry a slot, else by name — the frozen reply test.
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
    // Newest-first, as the frozen reader scans with `_ae_tac`.
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

    // The frozen output loop iterates the openings OLDEST-first (it reverses its
    // newest-first `order`).
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
