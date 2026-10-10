//! The `reply` helper: a tracked request's answer, created and delivered by the
//! core.
//!
//! The argv is
//! `[--as <agent>] <request-id> <message>`. A blank message is refused ON THE
//! MESSAGE — the delivered text `[<id>] <message>` is never blank, so the send
//! body's own guard cannot see this case, the case that once lost a whole
//! verdict. The request is the NEWEST `ask`/`review` carrying that ref, found
//! the way `requests` finds it. A request with a stored `target_slot` is
//! verified by SLOT AND SESSION against the replying pane — `--as` cannot
//! bypass that; it is display only, and merely warned about when it disagrees
//! with the stored name. A request without one (a pre-migration row, or a
//! slotless target) name-matches instead, with its own errors. The reply
//! goes to the asker's CURRENT pane — the stored `actor_slot` resolved in the
//! stored `actor_session` — and falls back to the stored display name only
//! when no stamped pane holds that slot. Then `[<id>] <message>` is pasted
//! and the `reply` event records the replier (`--as`, else the pane), the
//! routed target, the ref, the replier's slot and tmux session, the asker's
//! slot and session, and the message.
use std::io::{self, Write};
use std::path::Path;

use crate::requests::{Key, Pin, Request, Status, is_slot, states};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store;
use crate::time::Timestamp;
use crate::tmux::{ObservedSlot, ObservedViewer};
use crate::tracked::{self, EventFields};
use crate::transport;

/// The usage text.
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

/// The replying pane: its display ref (the stamp, `@<session>:`-prefixed when
/// the pane's tmux session is not this session's), its routing slot (the stamp
/// when it is in the slot grammar, else empty) and the pane's tmux session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replier {
    /// The display ref, or empty.
    pub display: String,
    /// The routing slot, or empty.
    pub slot: String,
    /// The pane's tmux session, or empty.
    pub session: String,
    /// That session's meta `session_id`, or empty.
    pub session_id: String,
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
            session_id: String::new(),
        }
    }
}

/// The newest `ask`/`review` carrying `id` in `dir`'s ledger — what
/// `ae_find_request` returns, read through [`states`] so the row is the one
/// `requests` shows.
#[must_use]
pub fn find(dir: &Path, id: &str) -> Option<Request> {
    let container = store::open(dir).container();
    states(&container)
        .into_iter()
        .find(|request| request.id == id.as_bytes())
}

/// The identity check's answer: who the reply is from, and the
/// advisory `--as` warning when that name disagrees with the stored target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// The `AE_SENDER_OVERRIDE` handed to `send`: `--as`,
    /// else the pane's display ref — possibly empty.
    pub sender: String,
    /// The warning line, newline included.
    pub warning: Option<String>,
}

/// The check, in its two branches.
///
/// # Errors
///
/// The error line, without its newline.
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
        // A target pinned by session id is that session whatever it is called
        // now; an unusable pin names nobody, so it never falls back to a name.
        let wrong_session = match request.to_session_id.pin() {
            Pin::None => me.session != target_session,
            Pin::Id(want) => me.session_id != want,
            Pin::Unusable => {
                return Err(format!(
                    "Error: request '{id}' records an unusable target session id; it cannot be verified"
                ));
            }
        };
        if me.slot != target_slot || wrong_session {
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

/// The slot resolver: the agent stamped on the pane holding
/// `want_slot` among `panes` (the roster of `want_session`, or of this session
/// when that is empty), spelled `@<session>:<agent>` when `want_session` is
/// another session.
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

/// The session the asker lives in NOW, when its record pins one by id:
/// `Ok(None)` for a legacy record, which keeps the stored name; the one
/// session recording the id otherwise — the replier's own included, since a
/// copied meta makes even that ambiguous. Missing, ambiguous or unusable
/// refuses naming the id — never a reused old name.
///
/// # Errors
///
/// The refusal line, without its newline.
pub fn asker_home(
    request: &Request,
    id: &str,
    lookup: impl FnOnce(&str) -> io::Result<Vec<String>>,
) -> Result<Option<(String, String)>, String> {
    let want = match request.from_session_id.pin() {
        Pin::None => return Ok(None),
        Pin::Unusable => {
            return Err(format!(
                "Error: request '{id}' records an unusable asker session id; not routed"
            ));
        }
        Pin::Id(want) => want,
    };
    let homes = lookup(want).map_err(|why| {
        format!("Error: request '{id}': cannot enumerate sessions to find id '{want}' ({why})")
    })?;
    match homes.as_slice() {
        [home] => Ok(Some((home.clone(), want.to_owned()))),
        [] => Err(format!(
            "Error: request '{id}' was asked from session id '{want}', which no session records now; not routed by its old name"
        )),
        many => Err(format!(
            "Error: request '{id}' was asked from session id '{want}', which {} sessions record ({}); refusing to guess",
            many.len(),
            many.join(", ")
        )),
    }
}

/// Where the reply goes, with the asker's session name and the id the record
/// pins: a legacy asker by [`route`]; a pinned one in the session holding its
/// id now — its slot's pane there, else its display name in that session.
///
/// # Errors
///
/// [`asker_home`]'s refusal line.
fn destination(
    dir: &Path,
    request: &Request,
    id: &str,
    own_session: &str,
) -> Result<(String, String, String), String> {
    let home = asker_home(request, id, |want| {
        dir.parent().and_then(Path::parent).map_or_else(
            || Ok(Vec::new()),
            |root| crate::lifecycle::sessions_with_id(root, want),
        )
    })?;
    // The asker's panes are enumerated on THAT session's recorded tmux server —
    // the same door `tracked::resolve` uses — never the ambient one.
    let roster = |search: &str| {
        tracked::named_server(dir, search, own_session)
            .ok()
            .and_then(|server| transport::observe_slots(&server, search))
    };
    Ok(match home {
        None => (
            route(request, own_session, roster),
            key_text(&request.from_session),
            String::new(),
        ),
        Some((home, want)) => {
            let panes = roster(&home).unwrap_or_default();
            let slot = key_text(&request.from_slot);
            let found = slot_resolve(&home, &slot, own_session, &panes).unwrap_or_else(|| {
                let stored = lossy(&request.from);
                if home == own_session {
                    stored
                } else {
                    format!("@{home}:{stored}")
                }
            });
            (found, home, want)
        }
    })
}

/// The destination re-proved just before delivery: a pinned asker is its slot
/// in the session recording its id NOW — anything else since the lookup is
/// not the asker, and says what it is. A legacy reply has no pin to re-prove.
fn not_the_asker(
    dir: &Path,
    resolved: &tracked::Resolved,
    slot: &str,
    id: &str,
    own_session: &str,
) -> Option<String> {
    (!id.is_empty()
        && (resolved.slot != slot
            || tracked::routing_id(dir, &resolved.slot, &resolved.session, own_session) != id))
        .then(|| {
            format!(
                "slot '{}' of session '{}' is not the asker (slot '{slot}', id '{id}')",
                resolved.slot, resolved.session
            )
        })
}

/// The order of refusals: usage, the blank body, the unknown id, the
/// identity check — each loud, each before anything is pasted.
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
            store::open(dir).events_path().display()
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
/// It is set after the slot check, and the send body's provenance envelope
/// takes it verbatim when it is non-empty.
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

/// Reply end to end.
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
    let mut me = Replier::from_observed(observed, own_session);
    me.session_id = tracked::routing_id(dir, &me.slot, &me.session, own_session);
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
    let (reply_target, target_session, target_id) =
        match destination(dir, &request, &parsed.id, own_session) {
            Ok(found) => found,
            Err(line) => {
                writeln!(err, "{line}")?;
                return Ok(EXIT_FAILED);
            }
        };
    let target_slot = key_text(&request.from_slot);
    let mut fields = EventFields {
        actor_session_id: &me.session_id,
        target_session_id: &target_id,
        ..EventFields::new(
            now,
            &actor,
            ACTION,
            &reply_target,
            &parsed.id,
            &me.slot,
            &me.session,
            &target_slot,
            &target_session,
            &parsed.body,
            "",
        )
    };
    if tracked::is_external(&reply_target) {
        return record_for_sink(dir, &fields, &reply_target, &parsed, err);
    }
    let (resolved, server) = match tracked::resolve_on(&reply_target, own_session, dir) {
        Ok(resolved) => resolved,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(EXIT_FAILED);
        }
    };
    if let Some(line) = not_the_asker(dir, &resolved, &target_slot, &target_id, own_session) {
        writeln!(err, "Error: request '{}': {line}; not delivered", parsed.id)?;
        return Ok(EXIT_FAILED);
    }
    let cross_session = !me.session.is_empty() && resolved.session != me.session;
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
        // The VERIFIED sender, and never an inherited one: the slot check above
        // decided who this reply is from, so the envelope takes its answer
        // rather than the caller's environment.
        actor: &verified.sender,
        body: &message,
        shape: crate::deliver::Shape::Send,
        defer,
        composed: crate::tool::Composed::NONE,
    };
    fields.target = &target_name;
    let (delivery, outcome) =
        tracked::caller_across_cut(dir, || tracked::deliver_request(&request, err));
    let delivery = delivery?;
    fields = tracked::stamp_caller(&fields, &outcome);
    let cross_session = cross_session.then_some(tracked::CrossSession {
        caller: &me.session,
        target: &resolved.session,
    });
    tracked::record_tracked_delivery(dir, &fields, delivery, cross_session, err)
}

/// A reply to an event-only sink: record, paste nothing. The console reads
/// its answer whole from the body file, so a body that cannot be kept there
/// records nothing at all.
fn record_for_sink(
    dir: &Path,
    fields: &EventFields<'_>,
    sink: &str,
    parsed: &Parsed,
    err: &mut impl Write,
) -> io::Result<u8> {
    let body_file = if sink == tracked::CONSOLE_SINK {
        match console_body(dir, parsed) {
            Ok(path) => path,
            Err(why) => {
                writeln!(err, "ae: reply {} not recorded: {why}", parsed.id)?;
                return Ok(EXIT_FAILED);
            }
        }
    } else {
        String::new()
    };
    let fields = EventFields {
        body_file: &body_file,
        ..*fields
    };
    // No durable cut, so one observation at write time is the proof consumed.
    let outcome = tracked::CorrelationOutcome::from_observation(tracked::observe_caller(dir));
    let fields = tracked::stamp_caller(&fields, &outcome);
    if let Err(why) = store::open(dir).append_event(&tracked::event_line(&fields)) {
        writeln!(err, "ae: reply {} not recorded: {why}", parsed.id)?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
}

/// The most a console reply body may hold — the durable-retry bound spawn
/// already keeps.
pub const CONSOLE_BODY_CAP: usize = 65_536;

/// Keep a console reply's whole body beside the session, refusing one over
/// [`CONSOLE_BODY_CAP`] before anything is written.
fn console_body(dir: &Path, parsed: &Parsed) -> Result<String, String> {
    let size = parsed.body.len();
    if size > CONSOLE_BODY_CAP {
        return Err(format!(
            "a console reply body holds at most {CONSOLE_BODY_CAP} bytes, this one {size}"
        ));
    }
    crate::deliver::store_body(dir, &parsed.id, ACTION, &parsed.body)
        .map(|path| path.display().to_string())
        .map_err(|why| format!("its body could not be stored: {why}"))
}

/// Bytes as text: the comparison never sees an invalid byte from ae's own
/// writers.
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
    use super::{Parsed, Replier, Usage, asker_home, parse, route, slot_resolve, verify};
    use crate::requests::{Key, Request, Status};
    use crate::time::Timestamp;
    use crate::tmux::{ObservedSlot, ObservedViewer};
    use crate::tracked::{EventFields, record_tracked_delivery};

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
            from_session_id: Key::Absent,
            to_session_id: Key::Absent,
            recorded: None,
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
            session_id: String::new(),
        }
    }

    const ID_A: &str = "0199c0de-1111-4890-abcd-ef0123456789";
    const ID_B: &str = "0199c0de-2222-4890-abcd-ef0123456789";

    fn pinned(from_id: Key, to_id: Key) -> Request {
        let keys = [value("main"), value("old"), value("worker.0"), value("old")];
        Request {
            from_session_id: from_id,
            to_session_id: to_id,
            ..request("cl:lead", "cl:w", keys)
        }
    }

    #[test]
    fn a_target_pinned_by_id_is_verified_by_id_whatever_the_names_say() {
        let row = pinned(Key::Absent, value(ID_A));
        let renamed = Replier {
            session_id: ID_A.to_owned(),
            ..me("cl:w", "worker.0", "new")
        };
        assert!(verify(&row, "ae-1", None, &renamed, "new").is_ok());
        // The reused name is another session: its id refuses it.
        let reused = Replier {
            session_id: ID_B.to_owned(),
            ..me("cl:w", "worker.0", "old")
        };
        assert!(verify(&row, "ae-1", None, &reused, "old").is_err());
        for bad in [Key::Empty, value("OLD")] {
            let verdict = verify(&pinned(Key::Absent, bad), "ae-1", None, &renamed, "new");
            assert!(verdict.is_err_and(|line| line.contains("unusable")));
        }
    }

    #[test]
    fn the_asker_is_found_by_its_pinned_id_or_refused_naming_it() {
        let none = |_: &str| -> std::io::Result<Vec<String>> { Ok(Vec::new()) };
        let legacy = pinned(Key::Absent, Key::Absent);
        assert_eq!(asker_home(&legacy, "ae-1", none), Ok(None));
        let row = pinned(value(ID_A), value(ID_B));
        let one = |_: &str| Ok(vec!["new".to_owned()]);
        assert_eq!(
            asker_home(&row, "ae-1", one),
            Ok(Some(("new".to_owned(), ID_A.to_owned())))
        );
        let two = |_: &str| Ok(vec!["new".to_owned(), "dup".to_owned()]);
        let gone = std::io::Error::other("unreadable");
        for refused in [
            asker_home(&row, "ae-1", none),
            asker_home(&row, "ae-1", two),
            asker_home(&row, "ae-1", |_| Err(gone)),
        ] {
            assert!(refused.is_err_and(|line| line.contains(ID_A)));
        }
        let bad = pinned(Key::Empty, Key::Absent);
        assert!(asker_home(&bad, "ae-1", none).is_err());
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
            ..ObservedViewer::default()
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
            ..ObservedViewer::default()
        };
        assert_eq!(
            Replier::from_observed(Some(&unstamped), "s"),
            me("cl:w", "", "s"),
            "a slot outside the grammar is no slot (_valid_slot)"
        );
        assert_eq!(Replier::from_observed(None, "s"), Replier::default());
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test writes and reads its isolated event ledger"
    )]
    fn an_unconfirmed_reply_event_is_recorded_by_the_reply_path() {
        let dir = std::env::temp_dir().join(format!("ae-reply-unconfirmed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the isolated state directory");
        let id = "ae-20260827T071112Z-00000004";
        let body_file = dir.join("messages/reply.body.txt").display().to_string();
        let mut err = Vec::new();
        let fields = EventFields {
            ts: Timestamp::parse("2026-08-27T07:11:12Z").expect("the timestamp parses"),
            actor: "worker",
            action: "reply",
            target: "lead",
            reference: id,
            actor_slot: "worker.0",
            actor_session: "session",
            target_slot: "main",
            target_session: "session",
            actor_session_id: "",
            target_session_id: "",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary: "the answer",
            body_file: "",
        };
        let code = record_tracked_delivery(
            &dir,
            &fields,
            Err(crate::deliver::Failure::Unconfirmed {
                body_file: body_file.clone(),
                framed: "⟦ae:msg from worker⟧\n[ae-1] the answer".to_owned(),
                notice: false,
            }),
            None,
            &mut err,
        )
        .expect("the unconfirmed reply event is recorded");
        assert_eq!(code, 0);
        assert!(
            String::from_utf8(err)
                .expect("the pending notice is utf-8")
                .contains("reply ae-20260827T071112Z-00000004 recorded as pending")
        );
        let event = std::fs::read_to_string(dir.join("events.jsonl")).expect("the event ledger");
        assert!(event.contains("\"action\":\"reply\""));
        assert!(event.contains("\"summary\":\"[unconfirmed] the answer\""));
        assert!(event.contains(&format!("\"body_file\":\"{body_file}\"")));

        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test reads back the event ledger the production reply wrote"
    )]
    fn reply_run_stamps_caller_on_the_production_writer() {
        use crate::inventory::ServerId;
        use crate::time::Timestamp;
        use crate::tmux::ObservedViewer;
        use crate::tracked::{self, IdentityTriple, Kind, Resolved, Sender};
        tracked::clear_test_hooks();
        let root = std::env::temp_dir().join(format!("ae-reply-run.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sessions").join("s");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            "session_id=1b4e28ba-2fa1-11d2-883f-0016d3cc4321\n",
        )
        .unwrap();
        let ts = Timestamp::parse("2026-08-27T07:11:12Z").unwrap();
        let uuid = "1b4e28ba-2fa1-11d2-883f-0016d3cc4321";
        let held = IdentityTriple {
            server: "/tmp/ae".to_owned(),
            pane: "%9".to_owned(),
            session_uuid: uuid.to_owned(),
        };
        let resolved = Resolved {
            pane: "%9".to_owned(),
            agent: "w".to_owned(),
            slot: "worker.0".to_owned(),
            session: "s".to_owned(),
        };
        let delivered = || crate::deliver::Delivered {
            body_file: String::new(),
            framed: "q".to_owned(),
            verification: crate::deliver::DeliveryVerification::Verified,
        };
        tracked::queue_observe(Ok(held.clone()));
        tracked::queue_observe(Ok(held.clone()));
        tracked::set_test_resolve(resolved.clone(), ServerId::Ambient);
        tracked::set_test_delivery(Ok(delivered()));
        let sender = Sender {
            display: "lead".to_owned(),
            slot: "main".to_owned(),
            session: "s".to_owned(),
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = tracked::run(
            Kind::Ask,
            &dir,
            &["w".to_owned(), "q".to_owned()],
            Some(&sender),
            "s",
            ts,
            1,
            crate::deliver::DEFAULT_DEFER,
            &mut out,
            &mut err,
        )
        .expect("ask");
        assert_eq!(code, 0, "ask {}", String::from_utf8_lossy(&err));
        let id = tracked::request_id("ae", ts, 1);
        tracked::queue_observe(Ok(held.clone()));
        tracked::queue_observe(Ok(held));
        let asker = Resolved {
            pane: "%3".to_owned(),
            agent: "lead".to_owned(),
            slot: "main".to_owned(),
            ..resolved
        };
        tracked::set_test_resolve(asker, ServerId::Ambient);
        tracked::set_test_delivery(Ok(crate::deliver::Delivered {
            body_file: String::new(),
            framed: "ans".to_owned(),
            verification: crate::deliver::DeliveryVerification::Verified,
        }));
        let observed = ObservedViewer {
            slot: Some("worker.0".into()),
            session: Some("s".into()),
            agent: Some("w".into()),
            ..ObservedViewer::default()
        };
        let mut err = Vec::new();
        let code = super::run(
            &dir,
            &[id, "ans".to_owned()],
            Some(&observed),
            "s",
            ts,
            crate::deliver::DEFAULT_DEFER,
            &mut err,
        )
        .expect("reply run");
        assert_eq!(code, 0, "reply {}", String::from_utf8_lossy(&err));
        let events = std::fs::read_to_string(dir.join("events.jsonl")).expect("events");
        assert!(events.contains(r#""action":"reply""#), "{events}");
        assert!(
            events.contains(r#""caller_server":"/tmp/ae""#),
            "deleting stamp_caller from reply::run must drop this: {events}"
        );
        tracked::clear_test_hooks();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A session directory holding ONE ask from `asker` to `lead` (`main` in
    /// `s`), as a pane-less sender records it; returns the dir and the id.
    #[allow(
        clippy::disallowed_methods,
        reason = "the test plants the journal the production reply reads"
    )]
    fn one_ask_from(asker: &str, tag: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("ae-reply-{tag}.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            "session_id=1b4e28ba-2fa1-11d2-883f-0016d3cc4321\n",
        )
        .unwrap();
        let id = "ae-20260930T120000Z-0000abcd".to_owned();
        std::fs::write(
            dir.join("events.jsonl"),
            format!(
                "{{\"ts\":\"2026-09-30T12:00:00Z\",\"actor\":\"{asker}\",\"action\":\"ask\",\"target\":\"lead\",\"ref\":\"{id}\",\"target_slot\":\"main\",\"target_session\":\"s\",\"summary\":\"q\"}}\n"
            ),
        )
        .unwrap();
        (dir, id)
    }

    /// `lead` in `main` replies `body` to `id`, its caller triple observed
    /// once, as the event-only arm consumes it.
    fn lead_replies(dir: &std::path::Path, id: &str, body: &str) -> (u8, String) {
        crate::tracked::clear_test_hooks();
        crate::tracked::queue_observe(Ok(crate::tracked::IdentityTriple {
            server: "/tmp/ae".to_owned(),
            pane: "%3".to_owned(),
            session_uuid: "1b4e28ba-2fa1-11d2-883f-0016d3cc4321".to_owned(),
        }));
        let observed = ObservedViewer {
            slot: Some("main".into()),
            session: Some("s".into()),
            agent: Some("lead".into()),
            ..ObservedViewer::default()
        };
        let mut err = Vec::new();
        let code = super::run(
            dir,
            &[id.to_owned(), body.to_owned()],
            Some(&observed),
            "s",
            Timestamp::parse("2026-09-30T12:01:00Z").unwrap(),
            crate::deliver::DEFAULT_DEFER,
            &mut err,
        )
        .expect("reply run");
        crate::tracked::clear_test_hooks();
        (code, String::from_utf8_lossy(&err).into_owned())
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "the test reads back what the production reply wrote"
    )]
    fn journal(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "the test reads back the body file the production reply stored"
    )]
    fn body_of(line: &str) -> Option<Vec<u8>> {
        let path = line.split("\"body_file\":\"").nth(1)?.split('"').next()?;
        std::fs::read(path).ok()
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "the test lists what the production reply stored"
    )]
    fn stored(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir.join("messages")).map_or(0, Iterator::count)
    }

    #[test]
    fn a_console_reply_keeps_its_whole_body_and_still_closes_the_request() {
        let (dir, id) = one_ask_from("console:local", "console-body");
        let body = format!(
            "first line é 中文 🎉\n    indented code\n\ttabbed\n\n{}",
            "ü".repeat(700)
        );
        assert_eq!(lead_replies(&dir, &id, &body), (0, String::new()));
        let events = journal(&dir);
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(events[1].contains("\"action\":\"reply\""), "{}", events[1]);
        assert!(
            events[1].contains("\"target\":\"console:local\""),
            "{}",
            events[1]
        );
        assert_eq!(body_of(&events[1]), Some(body.into_bytes()));
        assert_eq!(
            super::find(&dir, &id).map(|request| request.status),
            Some(Status::Replied)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_console_reply_over_64_kib_is_refused_before_anything_is_written() {
        let (dir, id) = one_ask_from("console:local", "console-cap");
        let exact = "a".repeat(65_536);
        assert_eq!(lead_replies(&dir, &id, &exact).0, 0);
        assert_eq!(body_of(&journal(&dir)[1]), Some(exact.into_bytes()));
        let (dir, id) = one_ask_from("console:local", "console-cap");
        let before = journal(&dir);
        let (code, err) = lead_replies(&dir, &id, &"a".repeat(65_537));
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("65536"), "{err}");
        assert_eq!(journal(&dir), before);
        assert_eq!(stored(&dir), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test plants a node where the body store must create its directory"
    )]
    fn a_console_reply_whose_body_cannot_be_stored_records_nothing() {
        let (dir, id) = one_ask_from("console:local", "console-store");
        std::fs::write(dir.join("messages"), "not a directory").unwrap();
        let before = journal(&dir);
        let (code, err) = lead_replies(&dir, &id, "the answer");
        assert_eq!(code, 1, "{err}");
        assert!(err.contains(&id), "{err}");
        assert_eq!(journal(&dir), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chat_bridge_reply_stays_an_event_only_record() {
        let (dir, id) = one_ask_from("telegram:42", "bridge-bytes");
        assert_eq!(lead_replies(&dir, &id, "the answer"), (0, String::new()));
        let events = journal(&dir);
        assert_eq!(
            events[1],
            format!(
                "{{\"ts\":\"2026-09-30T12:01:00Z\",\"actor\":\"lead\",\"action\":\"reply\",\"target\":\"telegram:42\",\"ref\":\"{id}\",\"actor_slot\":\"main\",\"actor_session\":\"s\",\"actor_session_id\":\"1b4e28ba-2fa1-11d2-883f-0016d3cc4321\",\"caller_server\":\"/tmp/ae\",\"caller_pane\":\"%3\",\"caller_session_uuid\":\"1b4e28ba-2fa1-11d2-883f-0016d3cc4321\",\"summary\":\"the answer\"}}"
            )
        );
        assert_eq!(stored(&dir), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
