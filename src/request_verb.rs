//! The two RECORD-ONLY request verbs: `withdraw <request-id> <reason…>`, the
//! asker closing its own open ask or review, and `accept <request-id>
//! <note…>`, the target taking it on while the reply stays owed.
//!
//! Neither pastes anything. Each appends ONE event to the ledger the request
//! lives in — a `cancel`, the ledger's existing withdrawal, or an
//! [`crate::session::ACCEPT`] — under that ledger's lock, after reading it
//! again there, and only when the request sensor
//! ([`crate::requests::states_in`]) would read the new record as doing what the
//! verb says: a record the ledger would ignore is refused, never claimed.
//!
//! Who may act is the routing key, never a display name: the asker withdraws
//! ([`Request::sent_by`]), the target accepts ([`crate::reply::verify`], the
//! reply's own check, because who may answer a request is who may accept it).
//!
//! A cross-session request has a copy in the other participant's ledger, and
//! the SAME line is appended there next. When that copy cannot be written the
//! verb says what landed and exits 1; running the same command again copies
//! the ORIGINAL line — never a second record, whatever the new reason says.
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::events::SessionKey;
use crate::reply::Replier;
use crate::requests::{Key, Pin, Request, Status, states_in};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store::{self, SourceRead};
use crate::time::Timestamp;
use crate::tmux::ObservedViewer;
use crate::tracked::{self, EventFields};

/// Which of the two verbs runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// The asker closes its own open request.
    Withdraw,
    /// The target takes the request on; it stays open until the reply.
    Accept,
}

impl Verb {
    /// The helper's name, which every line it prints begins with.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Withdraw => "withdraw",
            Self::Accept => "accept",
        }
    }

    /// The ledger action it records.
    #[must_use]
    pub const fn action(self) -> &'static str {
        match self {
            Self::Withdraw => "cancel",
            Self::Accept => crate::session::ACCEPT,
        }
    }

    /// The usage text.
    #[must_use]
    pub const fn usage(self) -> &'static str {
        match self {
            Self::Withdraw => {
                "Usage: withdraw <request-id> <reason>\n  Withdraw an ask/review you sent that is still open; the reason is recorded.\n"
            }
            Self::Accept => {
                "Usage: accept <request-id> <note>\n  Accept an ask/review sent to you: it leaves the unanswered count, and the reply is still owed.\n"
            }
        }
    }

    /// What the free text after the id is called.
    const fn text(self) -> &'static str {
        match self {
            Self::Withdraw => "reason",
            Self::Accept => "note",
        }
    }
}

/// The argv after the meta directory: the id, then the text — the remaining
/// words joined by one space. `None` is usage: no id, no text, or a text that
/// is only whitespace.
#[must_use]
pub fn parse(tail: &[String]) -> Option<(String, String)> {
    let (id, words) = tail.split_first()?;
    let text = words.join(" ");
    (!words.is_empty() && !tracked::is_blank(&text)).then(|| (id.clone(), text))
}

/// Run one verb end to end.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
#[allow(
    clippy::too_many_arguments,
    reason = "the helper's inputs as the dispatch reads them, spelled out rather than bundled"
)]
pub fn run(
    verb: Verb,
    dir: &Path,
    tail: &[String],
    observed: Option<&ObservedViewer>,
    own_session: &str,
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let word = verb.word();
    let Some((id, text)) = parse(tail) else {
        if tail.len() > 1 {
            writeln!(
                err,
                "ae: {word} REFUSED — the {} is empty (or only whitespace). Nothing was recorded.",
                verb.text()
            )?;
        }
        write!(err, "{}", verb.usage())?;
        return Ok(EXIT_USAGE);
    };
    let mut me = Replier::from_observed(observed, own_session);
    me.session_id = tracked::routing_id(dir, &me.slot, &me.session, own_session);
    if me.slot.is_empty() || me.display.is_empty() {
        writeln!(
            err,
            "Error: could not detect current agent identity; run {word} from your own ae pane"
        )?;
        return Ok(EXIT_FAILED);
    }
    let own_id = crate::meta::session_id_in(dir);
    let key = SessionKey::new(own_session, &own_id);
    let ledger = store::open(dir);
    let Some(request) = find(&ledger.container(), key, &id) else {
        writeln!(
            err,
            "Error: request id '{id}' not found in {}",
            ledger.events_path().display()
        )?;
        return Ok(EXIT_FAILED);
    };
    if let Err(line) = authorized(verb, &request, &id, &me, own_session) {
        writeln!(err, "{line}")?;
        return Ok(EXIT_FAILED);
    }
    let mirror = match other_ledger(dir, &request, own_session, &own_id) {
        Ok(mirror) => mirror,
        Err(why) => {
            writeln!(err, "{word}: {id} not recorded: {why}")?;
            return Ok(EXIT_FAILED);
        }
    };
    let repair_line = match (verb, request.status, &request.accepted) {
        (Verb::Withdraw, Status::Cancelled, _) => request.withdrawal.as_deref(),
        (Verb::Accept, Status::Pending, Some(accepted)) => Some(accepted.line.as_slice()),
        _ => None,
    };
    if let Err(why) =
        tracked::check_request_binding(dir, &request, repair_line, verb == Verb::Withdraw)
    {
        tracked::refuse_address(dir, &why, err)?;
        return Ok(EXIT_FAILED);
    }
    match (verb, request.status, &request.accepted) {
        (Verb::Withdraw, Status::Pending, _) | (Verb::Accept, Status::Pending, None) => {
            let line = record(verb, &request, &id, &text, &me, dir, now);
            write_new(verb, &ledger, key, &request, &line, mirror, out, err)
        }
        (Verb::Withdraw, Status::Cancelled, _) => match &request.withdrawal {
            Some(original) => repair(verb, &request, original, mirror, out, err),
            None => already(verb, &id, out, err),
        },
        (Verb::Accept, Status::Pending, Some(acceptance)) => {
            repair(verb, &request, &acceptance.line, mirror, out, err)
        }
        (_, status, _) => {
            writeln!(
                err,
                "Error: request '{id}' is already {}; nothing recorded",
                status.token()
            )?;
            Ok(EXIT_FAILED)
        }
    }
}

/// The newest opening of `id` in a ledger, as the sensor reads it.
fn find(container: &[u8], key: SessionKey<'_>, id: &str) -> Option<Request> {
    states_in(container, key)
        .into_iter()
        .find(|request| request.id == id.as_bytes())
}

/// Whether two rows are the SAME opening: what it is, when, and both ends by
/// display and routing key — the bytes a cross-session ask writes into both
/// ledgers alike. A newer opening that reuses the id is another request.
fn same_opening(left: &Request, right: &Request) -> bool {
    left.id == right.id
        && left.kind == right.kind
        && left.at == right.at
        && left.from == right.from
        && left.to == right.to
        && left.from_slot == right.from_slot
        && left.to_slot == right.to_slot
        && left.from_session == right.from_session
        && left.to_session == right.to_session
        && left.from_session_id == right.from_session_id
        && left.to_session_id == right.to_session_id
}

/// The sensor reads `line`, appended to `bytes`, as this verb's result for
/// `id`'s newest opening.
fn takes_effect(verb: Verb, bytes: Vec<u8>, line: &str, key: SessionKey<'_>, id: &str) -> bool {
    let mut after = bytes;
    after.extend_from_slice(line.as_bytes());
    find(&after, key, id).is_some_and(|request| took(verb, &request))
}

/// What the ledger would not read the record as.
const fn effect(verb: Verb) -> &'static str {
    match verb {
        Verb::Withdraw => "a withdrawal",
        Verb::Accept => "an acceptance",
    }
}

/// Whether the calling pane is the party this verb belongs to: the asker for
/// a withdrawal, the target for an acceptance — by routing key.
fn authorized(
    verb: Verb,
    request: &Request,
    id: &str,
    me: &Replier,
    own_session: &str,
) -> Result<(), String> {
    match verb {
        Verb::Accept => crate::reply::verify(request, id, None, me, own_session).map(|_| ()),
        Verb::Withdraw => {
            if request.sent_by(SessionKey::new(&me.session, &me.session_id), &me.slot) {
                return Ok(());
            }
            Err(format!(
                "Error: request '{id}' was asked by '{}' (slot '{}'@'{}'), current pane is slot '{}'@'{}'; only its asker may withdraw it",
                lossy(&request.from),
                key_text(&request.from_slot),
                key_text(&request.from_session),
                me.slot,
                me.session,
            ))
        }
    }
}

/// The record, `\n` included: the caller as actor, the OTHER party as target
/// with the routing key the request recorded for it, the text as summary, and
/// the calling pane's incarnation stamped beside them.
fn record(
    verb: Verb,
    request: &Request,
    id: &str,
    text: &str,
    me: &Replier,
    dir: &Path,
    now: Timestamp,
) -> String {
    let (target, slot, session, session_id) = match verb {
        Verb::Withdraw => (
            &request.to,
            &request.to_slot,
            &request.to_session,
            &request.to_session_id,
        ),
        Verb::Accept => (
            &request.from,
            &request.from_slot,
            &request.from_session,
            &request.from_session_id,
        ),
    };
    let (target, slot, session, session_id) = (
        lossy(target),
        key_text(slot),
        key_text(session),
        key_text(session_id),
    );
    let fields = EventFields {
        actor_session_id: &me.session_id,
        target_session_id: &session_id,
        ..EventFields::new(
            now,
            &me.display,
            verb.action(),
            &target,
            id,
            &me.slot,
            &me.session,
            &slot,
            &session,
            text,
            "",
        )
    };
    // No delivery and so no durable cut: one observation at write time is the
    // proof consumed, as a reply to an event-only sink takes it.
    let outcome = tracked::CorrelationOutcome::from_observation(tracked::observe_caller(dir));
    tracked::event_line(&tracked::stamp_caller(&fields, &outcome))
}

/// Why the ledger, read again under its lock, took nothing.
enum Refused {
    /// The ledger could not be read.
    Unreadable(String),
    /// The request moved since it was judged: gone, or no longer due.
    Moved(String),
    /// The ledger's newest opening of the id is another request.
    Diverged,
    /// The sensor would not read the record as this verb.
    NoEffect,
}

/// Append a new record to this ledger, then copy the same line into the other
/// participant's.
#[allow(
    clippy::too_many_arguments,
    reason = "one verb's whole context, each argument a different fact"
)]
fn write_new(
    verb: Verb,
    ledger: &store::SessionStore,
    key: SessionKey<'_>,
    request: &Request,
    line: &str,
    mirror: Option<(String, PathBuf)>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let word = verb.word();
    let id = &lossy(&request.id);
    let appended = ledger.append_event_decided(|read| {
        let bytes = readable(read)?;
        match find(&bytes, key, id) {
            Some(now) if !same_opening(request, &now) => {
                return Err(Refused::Moved(
                    "its newest opening is another request now".to_owned(),
                ));
            }
            Some(now) if due(verb, &now) => {}
            Some(now) => return Err(Refused::Moved(format!("it is {} now", now.status_cell()))),
            None => return Err(Refused::Moved("its opening is gone".to_owned())),
        }
        if !takes_effect(verb, bytes, line, key, id) {
            return Err(Refused::NoEffect);
        }
        Ok(line.to_owned())
    });
    let why = match appended {
        Ok(Ok(())) => None,
        Ok(Err(Refused::Unreadable(why) | Refused::Moved(why))) => Some(why),
        Ok(Err(Refused::Diverged | Refused::NoEffect)) => Some(format!(
            "the ledger would not read it as {}; the request names no routing key for this pane",
            effect(verb)
        )),
        Err(why) => Some(why.to_string()),
    };
    if let Some(why) = why {
        writeln!(err, "{word}: {id} not recorded: {why}")?;
        return Ok(EXIT_FAILED);
    }
    let here = key.name;
    if let Some((other, other_dir)) = mirror
        && let Err(gap) = copy_into(verb, request, &other, &other_dir, line)
    {
        writeln!(err, "{word}: {id} recorded in {here}; {}", gap.says(&other))?;
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "{word}: {id} recorded{}", confirmation(verb))?;
    Ok(0)
}

/// Copy the ORIGINAL record into the other participant's ledger when its copy
/// still lacks it; otherwise there is nothing left to do.
fn repair(
    verb: Verb,
    request: &Request,
    original: &[u8],
    mirror: Option<(String, PathBuf)>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let word = verb.word();
    let id = &lossy(&request.id);
    let mut line = original.to_vec();
    line.push(b'\n');
    let (Some((other, other_dir)), Ok(line)) = (mirror, String::from_utf8(line)) else {
        return already(verb, id, out, err);
    };
    match copy_into(verb, request, &other, &other_dir, &line) {
        Ok(true) => {
            writeln!(out, "{word}: {id} copy completed in {other}")?;
            Ok(0)
        }
        Ok(false) => already(verb, id, out, err),
        Err(gap) => {
            writeln!(err, "{word}: {id} recorded; {}", gap.says(&other))?;
            Ok(EXIT_FAILED)
        }
    }
}

/// The idempotent answer: an acceptance stands (exit 0), a withdrawal is
/// already final (exit 1, the request is closed).
fn already(verb: Verb, id: &str, out: &mut impl Write, err: &mut impl Write) -> io::Result<u8> {
    match verb {
        Verb::Accept => {
            writeln!(out, "accept: {id} already accepted; nothing recorded")?;
            Ok(0)
        }
        Verb::Withdraw => {
            writeln!(
                err,
                "Error: request '{id}' is already cancelled; nothing recorded"
            )?;
            Ok(EXIT_FAILED)
        }
    }
}

/// Why the other participant's copy did not take the line.
enum CopyGap {
    /// The ledger could not be locked, read or appended: a retry may land it.
    Failed(String),
    /// Its copy is not this request, or would not read the line as this verb:
    /// nothing was written there, and a retry writes nothing either.
    Refused(String),
}

impl CopyGap {
    /// The words after the id, naming `other`.
    fn says(&self, other: &str) -> String {
        match self {
            Self::Failed(why) => format!(
                "copy into {other} failed: {why}; run the same command again to complete it"
            ),
            Self::Refused(why) => {
                format!("copy into {other} failed: refused: {why}; nothing was written there")
            }
        }
    }
}

/// Append `line` to the other participant's ledger when its copy of the
/// request is still due there, re-read under its lock: `Ok(true)` when it was
/// written, `Ok(false)` when that copy is absent or already past this verb. A
/// copy whose newest opening of the id is another request, or that would not
/// read the line as this verb, is refused and left untouched.
fn copy_into(
    verb: Verb,
    request: &Request,
    other: &str,
    other_dir: &Path,
    line: &str,
) -> Result<bool, CopyGap> {
    let id = &lossy(&request.id);
    let other_id = crate::meta::session_id_in(other_dir);
    let key = SessionKey::new(other, &other_id);
    let appended = store::open(other_dir).append_event_decided(|read| {
        let bytes = readable(read)?;
        match find(&bytes, key, id) {
            Some(copy) if !same_opening(request, &copy) => return Err(Refused::Diverged),
            Some(copy) if due(verb, &copy) => {}
            _ => return Err(Refused::Moved(String::new())),
        }
        if !takes_effect(verb, bytes, line, key, id) {
            return Err(Refused::NoEffect);
        }
        Ok(line.to_owned())
    });
    match appended {
        Ok(Ok(())) => Ok(true),
        Ok(Err(Refused::Moved(_))) => Ok(false),
        Ok(Err(Refused::Diverged)) => Err(CopyGap::Refused(
            "its newest opening of this id is another request".to_owned(),
        )),
        Ok(Err(Refused::NoEffect)) => Err(CopyGap::Refused(format!(
            "its ledger would not read the record as {}",
            effect(verb)
        ))),
        Ok(Err(Refused::Unreadable(why))) => Err(CopyGap::Failed(why)),
        Err(why) => Err(CopyGap::Failed(why.to_string())),
    }
}

/// A ledger's bytes, an absent one empty.
fn readable(read: SourceRead) -> Result<Vec<u8>, Refused> {
    match read {
        SourceRead::Ready(bytes) => Ok(bytes),
        SourceRead::Absent => Ok(Vec::new()),
        SourceRead::Invalid(why) | SourceRead::Unreadable(why) => Err(Refused::Unreadable(why)),
    }
}

/// Whether this verb still has work on `request`: open, and for an acceptance
/// not accepted yet.
fn due(verb: Verb, request: &Request) -> bool {
    request.status == Status::Pending && (verb == Verb::Withdraw || request.accepted.is_none())
}

/// Whether `request` reads as this verb's result.
fn took(verb: Verb, request: &Request) -> bool {
    match verb {
        Verb::Withdraw => request.status == Status::Cancelled,
        Verb::Accept => request.status == Status::Pending && request.accepted.is_some(),
    }
}

/// What the success line adds.
const fn confirmation(verb: Verb) -> &'static str {
    match verb {
        Verb::Withdraw => "; the request is closed",
        Verb::Accept => "; it stays open until you reply",
    }
}

/// The other participant's ledger of a cross-session request, by the session
/// that holds the id the request pinned for that side NOW — never a reused old
/// name. `None` when both sides are this session, or when no session records
/// the other side's id any more.
pub(crate) fn other_ledger(
    dir: &Path,
    request: &Request,
    own_session: &str,
    own_id: &str,
) -> Result<Option<(String, PathBuf)>, String> {
    let sides = [
        (&request.from_session, &request.from_session_id),
        (&request.to_session, &request.to_session_id),
    ];
    for (session, session_id) in sides {
        let name = key_text(session);
        let name = if name.is_empty() { own_session } else { &name };
        let other = match session_id.pin() {
            Pin::Id(id) if id == own_id => continue,
            Pin::None if name == own_session => continue,
            Pin::Unusable => return Err("the request records an unusable session id".to_owned()),
            Pin::None => name.to_owned(),
            Pin::Id(id) => {
                let root = dir.parent().and_then(Path::parent).unwrap_or(dir);
                let homes = crate::lifecycle::sessions_with_id(root, id).map_err(|why| {
                    format!("cannot enumerate sessions to find id '{id}' ({why})")
                })?;
                match homes.as_slice() {
                    [] => return Ok(None),
                    [home] => home.clone(),
                    many => {
                        return Err(format!(
                            "session id '{id}' is recorded by {} sessions ({}); refusing to guess",
                            many.len(),
                            many.join(", ")
                        ));
                    }
                }
            }
        };
        let other_dir = tracked::participant_dir(dir, &other).map_err(|why| why.to_string())?;
        return Ok(Some((other, other_dir)));
    }
    Ok(None)
}

/// Bytes as text: ae's own writers never store an invalid byte here.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A routing member's text: absent and empty are both empty.
fn key_text(key: &Key) -> String {
    key.value().map(lossy).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{Verb, parse};

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn the_text_is_every_word_after_the_id_and_never_blank() {
        assert_eq!(
            parse(&words(&["ae-1", "done", "elsewhere"])),
            Some(("ae-1".to_owned(), "done elsewhere".to_owned()))
        );
        for tail in [
            &[][..],
            &["ae-1"][..],
            &["ae-1", " \t"][..],
            &["ae-1", "", " "][..],
        ] {
            assert_eq!(parse(&words(tail)), None, "{tail:?}");
        }
    }

    #[test]
    fn each_verb_records_its_own_ledger_action() {
        assert_eq!(Verb::Withdraw.action(), "cancel");
        assert_eq!(Verb::Accept.action(), "accept");
        for verb in [Verb::Withdraw, Verb::Accept] {
            assert!(
                verb.usage()
                    .starts_with(&format!("Usage: {} ", verb.word()))
            );
        }
    }
}
