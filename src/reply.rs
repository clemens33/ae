//! The `reply` helper (P2.5b): a tracked request's answer, created and
//! delivered by the core, consuming what either the core (P2.5a) or the frozen
//! `ae_tracked_send` recorded.
//!
//! What the frozen `helper_reply_main` does, kept exactly. The argv is
//! `[--as <agent>] <request-id> <message>`. A blank message is refused ON THE
//! MESSAGE — the delivered text `[<id>] <message>` is never blank, so the send
//! body's own guard cannot see this case, the case that once lost a whole
//! verdict. The request is the NEWEST `ask`/`review` carrying that ref, found
//! the way `requests` finds it. A request with a stored `target_slot` is
//! verified by SLOT AND SESSION against the replying pane — `--as` cannot
//! bypass that; it is display only, and merely warned about when it disagrees
//! with the stored name. A request without one (a pre-migration row, or a
//! slotless target) name-matches as before, with the frozen errors. The reply
//! goes to the asker's CURRENT pane — the stored `actor_slot` resolved in the
//! stored `actor_session` — and falls back to the stored display name only
//! when no stamped pane holds that slot. Then `[<id>] <message>` is pasted
//! and the `reply` event records the replier (`--as`, else the pane), the
//! routed target, the ref, the replier's slot and tmux session, the asker's
//! slot and session, and the message.
use std::io::{self, Write};
use std::path::Path;

use crate::event_text::{CONTAINER, read_container};
use crate::requests::{Key, Request, Status, is_slot, states};
use crate::state::{self, EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tmux::{ObservedSlot, ObservedViewer};
use crate::tracked::{self, EventFields};
use crate::transport;

/// The frozen usage text.
pub const USAGE: &str = "Usage: reply [--as <agent>] <request-id> <message>\n  Reply to a logged ask/review request using its request id.\n";

/// The event's `action`.
pub const ACTION: &str = "reply";

/// What the argv said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// `--as <agent>`, when given.
    pub as_name: Option<String>,
    /// The request id.
    pub id: String,
    /// The message: the remaining words joined by one space (`"$*"`).
    pub body: String,
}

/// A refused argv: [`USAGE`] to stderr, the usage exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage;

/// Parse the argv after the meta directory: fewer than two words is usage,
/// and `--as` needs an agent, an id and a message (`$# -ge 4`).
///
/// # Errors
///
/// [`Usage`].
pub fn parse(tail: &[String]) -> Result<Parsed, Usage> {
    match tail {
        [flag, rest @ ..] if flag == "--as" => match rest {
            [name, id, body @ ..] if !body.is_empty() => Ok(Parsed {
                as_name: Some(name.clone()),
                id: id.clone(),
                body: body.join(" "),
            }),
            _ => Err(Usage),
        },
        [id, body @ ..] if !body.is_empty() => Ok(Parsed {
            as_name: None,
            id: id.clone(),
            body: body.join(" "),
        }),
        _ => Err(Usage),
    }
}

/// The replying pane, as the frozen helper reads it: `ae_current_agent_ref`
/// (the stamp, `@<session>:`-prefixed when the pane's tmux session is not
/// this session's), `ae_current_slot` (the stamp when it is in the slot
/// grammar, else empty) and the pane's tmux session (`#S`). No pane at all is
/// three empty strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replier {
    /// The display ref, or empty.
    pub display: String,
    /// The routing slot, or empty.
    pub slot: String,
    /// The pane's tmux session, or empty.
    pub session: String,
}

impl Replier {
    /// Read the three off one pane observation.
    #[must_use]
    pub fn from_observed(observed: Option<&ObservedViewer>, own_session: &str) -> Self {
        let Some(observed) = observed else {
            return Self::default();
        };
        let session = observed.session.clone().unwrap_or_default();
        let agent = observed.agent.clone().unwrap_or_default();
        let display = if agent.is_empty() {
            String::new()
        } else if !session.is_empty() && session != own_session {
            format!("@{session}:{agent}")
        } else {
            agent
        };
        let slot = observed
            .slot
            .as_deref()
            .filter(|slot| is_slot(slot))
            .unwrap_or_default()
            .to_owned();
        Self {
            display,
            slot,
            session,
        }
    }
}

/// The newest `ask`/`review` carrying `id` in `dir`'s ledger — what
/// `ae_find_request` returns, read through [`states`] so the row is the one
/// `requests` shows. `None` for no such request, or no ledger at all.
#[must_use]
pub fn find(dir: &Path, id: &str) -> Option<Request> {
    let container = read_container(&dir.join(CONTAINER));
    states(&container)
        .into_iter()
        .find(|request| request.id == id.as_bytes())
}

/// The frozen identity check's answer: who the reply is from, and the
/// advisory `--as` warning when that name disagrees with the stored target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// The `AE_SENDER_OVERRIDE` the frozen helper hands to `send`: `--as`,
    /// else the pane's display ref — possibly empty.
    pub sender: String,
    /// The warning line, newline included.
    pub warning: Option<String>,
}

/// The frozen check, in its two branches.
///
/// # Errors
///
/// The frozen error line, without its newline.
pub fn verify(
    request: &Request,
    id: &str,
    as_name: Option<&str>,
    me: &Replier,
    own_session: &str,
) -> Result<Verified, String> {
    let target = lossy(&request.to);
    let as_name = as_name.filter(|name| !name.is_empty());
    let target_slot = key_text(&request.to_slot);
    if !target_slot.is_empty() {
        let stored_session = key_text(&request.to_session);
        let target_session = if stored_session.is_empty() {
            own_session
        } else {
            stored_session.as_str()
        };
        if me.slot != target_slot || me.session != target_session {
            let self_slot = if me.slot.is_empty() {
                "none"
            } else {
                me.slot.as_str()
            };
            return Err(format!(
                "Error: request '{id}' is assigned to slot '{target_slot}'@'{target_session}', current pane is slot '{self_slot}'@'{}'",
                me.session
            ));
        }
        let warning = as_name.filter(|name| *name != target).map(|name| {
            format!(
                "Warning: --as '{name}' != stored target name '{target}' (name is advisory; slot verified)\n"
            )
        });
        let sender = as_name.map_or_else(|| me.display.clone(), ToOwned::to_owned);
        return Ok(Verified { sender, warning });
    }
    if let Some(name) = as_name {
        if name != target {
            return Err(format!(
                "Error: override agent '{name}' does not match assigned target '{target}'"
            ));
        }
        return Ok(Verified {
            sender: name.to_owned(),
            warning: None,
        });
    }
    if me.display.is_empty() {
        return Err(format!(
            "Error: could not detect current agent identity; rerun with --as '{target}' from the assigned agent context"
        ));
    }
    if me.display != target {
        return Err(format!(
            "Error: request '{id}' is assigned to '{target}', current pane is '{}'",
            me.display
        ));
    }
    Ok(Verified {
        sender: me.display.clone(),
        warning: None,
    })
}

/// The frozen `ae_slot_resolver`: the agent stamped on the pane holding
/// `want_slot` among `panes` (the roster of `want_session`, or of this session
/// when that is empty), spelled `@<session>:<agent>` when `want_session` is
/// another session. `None` when no stamped pane holds the slot — the caller
/// then keeps the stored name.
#[must_use]
pub fn slot_resolve(
    want_session: &str,
    want_slot: &str,
    own_session: &str,
    panes: &[ObservedSlot],
) -> Option<String> {
    if want_slot.is_empty() {
        return None;
    }
    let pane = panes
        .iter()
        .find(|pane| pane.slot == want_slot && !pane.agent.is_empty())?;
    Some(if !want_session.is_empty() && want_session != own_session {
        format!("@{want_session}:{}", pane.agent)
    } else {
        pane.agent.clone()
    })
}

/// Where the reply goes: the asker's current pane by its stored slot (looked
/// up through `roster`, which enumerates one session's panes or fails), else
/// the stored display name.
#[must_use]
pub fn route(
    request: &Request,
    own_session: &str,
    roster: impl FnOnce(&str) -> Option<Vec<ObservedSlot>>,
) -> String {
    let stored = lossy(&request.from);
    let want_slot = key_text(&request.from_slot);
    if want_slot.is_empty() {
        return stored;
    }
    let want_session = key_text(&request.from_session);
    let search = if want_session.is_empty() {
        own_session
    } else {
        want_session.as_str()
    };
    let panes = roster(search).unwrap_or_default();
    slot_resolve(&want_session, &want_slot, own_session, &panes).unwrap_or(stored)
}

/// The frozen order of refusals: usage, the blank body, the unknown id, the
/// identity check — each loud, each before anything is pasted. `Ok(Err(code))`
/// is a refusal whose text is already on stderr; `Ok(Ok(..))` is the argv, the
/// request it names and who the reply is from.
fn admit(
    dir: &Path,
    tail: &[String],
    me: &Replier,
    own_session: &str,
    err: &mut impl Write,
) -> io::Result<Result<(Parsed, Request, Verified), u8>> {
    let Ok(parsed) = parse(tail) else {
        write!(err, "{USAGE}")?;
        return Ok(Err(EXIT_USAGE));
    };
    if tracked::is_blank(&parsed.body) {
        write!(err, "{}", tracked::refusal(ACTION))?;
        return Ok(Err(EXIT_FAILED));
    }
    let Some(request) = find(dir, &parsed.id) else {
        writeln!(
            err,
            "Error: request id '{}' not found in {}",
            parsed.id,
            dir.join(CONTAINER).display()
        )?;
        return Ok(Err(EXIT_FAILED));
    };
    match verify(
        &request,
        &parsed.id,
        parsed.as_name.as_deref(),
        me,
        own_session,
    ) {
        Ok(verified) => Ok(Ok((parsed, request, verified))),
        Err(line) => {
            writeln!(err, "{line}")?;
            Ok(Err(EXIT_FAILED))
        }
    }
}

/// The environment a reply that still runs a HELPER would hand it: the
/// VERIFIED sender as `AE_SENDER_OVERRIDE` — always set, so an override
/// inherited from the caller's environment is overwritten, empty included.
/// The frozen helper `exec env AE_SENDER_OVERRIDE="$reply_sender"` after the
/// slot check, and the send body's provenance envelope took that variable
/// verbatim when it was non-empty.
///
/// Since B move 1 the reply pastes for itself and hands the envelope the
/// verified sender directly, so nothing reads this on the reply path. It
/// stays as the written form of that rule. Leaving it to inheritance would
/// let
/// `AE_SENDER_OVERRIDE=spoof reply …` envelope the delivery as `spoof` after
/// the slot had verified someone else — and would envelope a `--as` reply
/// from the physical pane while the event named the `--as` actor. Plus the
/// action and ref the body store names the recovery file after.
///
/// ```
/// use ae::reply::delivery_env;
///
/// assert_eq!(
///     delivery_env("", "ae-1"),
///     [("AE_SENDER_OVERRIDE", ""), ("_AE_EVENT_ACTION", "reply"), ("_AE_EVENT_REF", "ae-1")],
///     "an empty sender still overwrites whatever was inherited"
/// );
/// ```
#[must_use]
pub fn delivery_env<'a>(sender: &'a str, id: &'a str) -> [(&'a str, &'a str); 3] {
    [
        ("AE_SENDER_OVERRIDE", sender),
        ("_AE_EVENT_ACTION", ACTION),
        ("_AE_EVENT_REF", id),
    ]
}

/// Reply end to end. `observed` is the replying pane (none for no pane);
/// `own_session` is this session's name as P2.1b derives it. Nothing is
/// printed on success — the frozen reply prints nothing.
///
/// # Errors
///
/// Only a failure to write `err`.
pub fn run(
    dir: &Path,
    tail: &[String],
    observed: Option<&ObservedViewer>,
    own_session: &str,
    now: Timestamp,
    defer: std::time::Duration,
    err: &mut impl Write,
) -> io::Result<u8> {
    let me = Replier::from_observed(observed, own_session);
    let (parsed, request, verified) = match admit(dir, tail, &me, own_session, err)? {
        Ok(admitted) => admitted,
        Err(code) => return Ok(code),
    };
    if let Some(warning) = &verified.warning {
        write!(err, "{warning}")?;
    }
    if request.status == Status::Replied {
        writeln!(
            err,
            "Note: request '{}' already has a reply on file; delivering this one as a follow-up",
            parsed.id
        )?;
    }
    // The emitter's own fallback for an empty AE_SENDER_OVERRIDE and no stamp.
    let actor = if verified.sender.is_empty() {
        "human".to_owned()
    } else {
        verified.sender.clone()
    };
    // The asker's panes are enumerated on THAT session's recorded tmux server —
    // the same door `tracked::resolve` uses — never the ambient one. Under the
    // ambient server the roster came back empty whenever the session lived on
    let reply_target = route(&request, own_session, |search| {
        tracked::named_server(dir, search, own_session)
            .ok()
            .and_then(|server| transport::observe_slots(&server, search))
    });
    let target_slot = key_text(&request.from_slot);
    let target_session = key_text(&request.from_session);
    let mut fields = EventFields {
        ts: now,
        actor: &actor,
        action: ACTION,
        target: &reply_target,
        reference: &parsed.id,
        actor_slot: &me.slot,
        actor_session: &me.session,
        target_slot: &target_slot,
        target_session: &target_session,
        summary: &parsed.body,
        body_file: "",
    };
    if tracked::is_external(&reply_target) {
        // An event-only sink: the frozen send records and pastes nothing.
        if let Err(why) = state::emit(dir, &tracked::event_line(&fields)) {
            writeln!(err, "ae: reply {} not recorded: {why}", parsed.id)?;
            return Ok(EXIT_FAILED);
        }
        return Ok(0);
    }
    let (resolved, server) = match tracked::resolve_on(&reply_target, own_session, dir) {
        Ok(resolved) => resolved,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(EXIT_FAILED);
        }
    };
    let target_name = if resolved.agent.is_empty() {
        reply_target.clone()
    } else {
        resolved.agent.clone()
    };
    let message = format!("[{}] {}", parsed.id, parsed.body);
    let request = crate::deliver::Request {
        dir,
        server: &server,
        pane: &resolved.pane,
        logged_target: &target_name,
        target_session: &resolved.session,
        pane_slot: &resolved.slot,
        own_session,
        action: ACTION,
        reference: &parsed.id,
        // The VERIFIED sender, and never an inherited one: the slot check
        // above decided who this reply is from, so the envelope takes its
        // answer rather than the caller's environment. A `--as` reply is
        actor: &verified.sender,
        body: &message,
        shape: crate::deliver::Shape::Send,
        defer,
    };
    let Ok(delivered) = crate::deliver::deliver(&request, err)? else {
        // Every arm of a refused delivery has already said what happened and
        // where the body is; nothing is recorded for one.
        return Ok(EXIT_FAILED);
    };
    fields.target = &target_name;
    fields.body_file = &delivered.body_file;
    if let Err(why) = state::emit(dir, &tracked::event_line(&fields)) {
        writeln!(
            err,
            "ae: reply {} was delivered to {target_name} but its event was not emitted: {why}",
            parsed.id
        )?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
}

/// Bytes as text, the way the frozen helper compares them: it never sees an
/// invalid byte from its own writers.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A routing member's text: absent and empty are both empty here, as the
/// frozen `read` leaves the variable empty for either.
fn key_text(key: &Key) -> String {
    key.value().map(lossy).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{Parsed, Replier, Usage, parse, route, slot_resolve, verify};
    use crate::requests::{Key, Request, Status};
    use crate::tmux::{ObservedSlot, ObservedViewer};

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn request(from: &str, to: &str, keys: [Key; 4]) -> Request {
        let [from_slot, from_session, to_slot, to_session] = keys;
        Request {
            status: Status::Pending,
            kind: b"ask".to_vec(),
            id: b"ae-1".to_vec(),
            from: from.as_bytes().to_vec(),
            to: to.as_bytes().to_vec(),
            at: Vec::new(),
            body_file: Vec::new(),
            from_slot,
            to_slot,
            from_session,
            to_session,
            summary: Vec::new(),
        }
    }

    fn value(text: &str) -> Key {
        Key::Value(text.as_bytes().to_vec())
    }

    fn me(display: &str, slot: &str, session: &str) -> Replier {
        Replier {
            display: display.to_owned(),
            slot: slot.to_owned(),
            session: session.to_owned(),
        }
    }

    #[test]
    fn argv_reads_as_the_helper_reads_it() {
        assert_eq!(parse(&[]), Err(Usage));
        assert_eq!(parse(&words(&["ae-1"])), Err(Usage), "$# -lt 2");
        assert_eq!(
            parse(&words(&["ae-1", "the", "answer"])),
            Ok(Parsed {
                as_name: None,
                id: "ae-1".into(),
                body: "the answer".into()
            })
        );
        assert_eq!(
            parse(&words(&["--as", "cl:x", "ae-1"])),
            Err(Usage),
            "--as needs four words"
        );
        assert_eq!(
            parse(&words(&["--as", "cl:x", "ae-1", "ok"])),
            Ok(Parsed {
                as_name: Some("cl:x".into()),
                id: "ae-1".into(),
                body: "ok".into()
            })
        );
    }

    #[test]
    fn the_replier_is_read_as_the_three_frozen_readers_read_it() {
        let observed = ObservedViewer {
            slot: Some("worker.0".into()),
            session: Some("s".into()),
            agent: Some("cl:w".into()),
        };
        assert_eq!(
            Replier::from_observed(Some(&observed), "s"),
            me("cl:w", "worker.0", "s")
        );
        assert_eq!(
            Replier::from_observed(Some(&observed), "other"),
            me("@s:cl:w", "worker.0", "s"),
            "another session's pane is spelled with its session"
        );
        let unstamped = ObservedViewer {
            slot: Some("not-a-slot".into()),
            session: Some("s".into()),
            agent: Some("cl:w".into()),
        };
        assert_eq!(
            Replier::from_observed(Some(&unstamped), "s"),
            me("cl:w", "", "s"),
            "a slot outside the grammar is no slot (_valid_slot)"
        );
        assert_eq!(Replier::from_observed(None, "s"), Replier::default());
    }

    #[test]
    fn a_slotted_request_is_verified_by_slot_and_session_and_as_is_advisory() {
        let slotted = request(
            "cl:lead",
            "cl:w",
            [value("main"), value("s"), value("worker.0"), value("s")],
        );
        let ok = verify(&slotted, "ae-1", None, &me("cl:w", "worker.0", "s"), "s").unwrap();
        assert_eq!((ok.sender.as_str(), ok.warning), ("cl:w", None));
        let renamed = verify(
            &slotted,
            "ae-1",
            Some("stale:old"),
            &me("cl:w2", "worker.0", "s"),
            "s",
        )
        .unwrap();
        assert_eq!(renamed.sender, "stale:old", "--as is the display");
        assert_eq!(
            renamed.warning.as_deref(),
            Some(
                "Warning: --as 'stale:old' != stored target name 'cl:w' (name is advisory; slot verified)\n"
            )
        );
        assert_eq!(
            verify(&slotted, "ae-1", Some("cl:w"), &me("cl:lead", "main", "s"), "s"),
            Err(
                "Error: request 'ae-1' is assigned to slot 'worker.0'@'s', current pane is slot 'main'@'s'"
                    .to_owned()
            ),
            "--as cannot bypass the slot"
        );
        assert_eq!(
            verify(&slotted, "ae-1", None, &me("", "", ""), "s"),
            Err(
                "Error: request 'ae-1' is assigned to slot 'worker.0'@'s', current pane is slot 'none'@''"
                    .to_owned()
            )
        );
        // R1: an empty stored session defaults to THIS session, never fails open.
        let no_session = request(
            "cl:lead",
            "cl:w",
            [value("main"), value("s"), value("worker.0"), Key::Empty],
        );
        assert!(
            verify(
                &no_session,
                "ae-1",
                None,
                &me("cl:w", "worker.0", "other"),
                "s"
            )
            .is_err()
        );
        assert!(verify(&no_session, "ae-1", None, &me("cl:w", "worker.0", "s"), "s").is_ok());
        // An empty --as is no --as.
        let empty_as = verify(
            &slotted,
            "ae-1",
            Some(""),
            &me("cl:w", "worker.0", "s"),
            "s",
        )
        .unwrap();
        assert_eq!((empty_as.sender.as_str(), empty_as.warning), ("cl:w", None));
    }

    #[test]
    fn an_unslotted_request_name_matches_with_the_frozen_errors() {
        let old = request(
            "a:b",
            "c:d",
            [Key::Absent, Key::Absent, Key::Absent, Key::Absent],
        );
        assert_eq!(
            verify(&old, "ae-1", Some("x:y"), &me("c:d", "worker.0", "s"), "s"),
            Err("Error: override agent 'x:y' does not match assigned target 'c:d'".to_owned())
        );
        assert_eq!(
            verify(&old, "ae-1", Some("c:d"), &me("", "", ""), "s")
                .unwrap()
                .sender,
            "c:d"
        );
        assert_eq!(
            verify(&old, "ae-1", None, &me("", "", ""), "s"),
            Err(
                "Error: could not detect current agent identity; rerun with --as 'c:d' from the assigned agent context"
                    .to_owned()
            )
        );
        assert_eq!(
            verify(&old, "ae-1", None, &me("e:f", "main", "s"), "s"),
            Err("Error: request 'ae-1' is assigned to 'c:d', current pane is 'e:f'".to_owned())
        );
        assert_eq!(
            verify(&old, "ae-1", None, &me("c:d", "main", "s"), "s")
                .unwrap()
                .sender,
            "c:d"
        );
        // A slotless TARGET on a modern row is the same branch.
        let slotless_target = request(
            "cl:lead",
            "human",
            [value("main"), value("s"), Key::Empty, Key::Empty],
        );
        assert!(verify(&slotless_target, "ae-1", None, &me("human", "", ""), "s").is_ok());
    }

    #[test]
    fn the_reply_is_routed_by_the_stored_slot_and_falls_back_to_the_stored_name() {
        let panes = vec![
            ObservedSlot {
                pane: "%1".into(),
                slot: "main".into(),
                agent: "renamed:lead".into(),
            },
            ObservedSlot {
                pane: "%2".into(),
                slot: "worker.0".into(),
                agent: String::new(),
            },
        ];
        assert_eq!(
            slot_resolve("s", "main", "s", &panes).as_deref(),
            Some("renamed:lead")
        );
        assert_eq!(
            slot_resolve("other", "main", "s", &panes).as_deref(),
            Some("@other:renamed:lead"),
            "another session is spelled"
        );
        assert_eq!(
            slot_resolve("", "main", "s", &panes).as_deref(),
            Some("renamed:lead"),
            "an empty session is this one"
        );
        assert_eq!(
            slot_resolve("s", "worker.0", "s", &panes),
            None,
            "an unstamped pane holding the slot resolves nothing"
        );
        assert_eq!(slot_resolve("s", "", "s", &panes), None);
        let asked = request(
            "cl:lead",
            "cl:w",
            [value("main"), value("s"), value("worker.0"), value("s")],
        );
        let searched = std::cell::RefCell::new(String::new());
        let routed = route(&asked, "s", |session| {
            *searched.borrow_mut() = session.to_owned();
            Some(panes.clone())
        });
        assert_eq!(
            (routed.as_str(), searched.borrow().as_str()),
            ("renamed:lead", "s")
        );
        assert_eq!(
            route(&asked, "s", |_| None),
            "cl:lead",
            "a roster that cannot be read keeps the stored name"
        );
        let old = request(
            "a:b",
            "c:d",
            [Key::Absent, Key::Absent, Key::Absent, Key::Absent],
        );
        assert_eq!(route(&old, "s", |_| panic!("no slot, no lookup")), "a:b");
    }
}
