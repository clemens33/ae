//! `/open <agent>`: the proof that a seat's pane is still the one the chat
//! names, and the one tmux list that selects it inside the chat's own session.
//!
//! PURE — the reads arrive as arguments. The proof is separate from the act so
//! a reader with no tmux client of its own can reuse it.

use crate::console::needs::SeatRef;
use crate::inventory::ServerId;
use crate::meta::RosterEntry;
use crate::tmux::{ObservedClient, ObservedSlot, PickerPane, WindowPane};

/// A seat's pane, proven.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub seat: SeatRef,
    /// The session's `$<n>` id.
    pub session_id: String,
    /// The pane's `%<n>` id.
    pub pane: String,
    /// The session uuid the chat is bound to.
    pub uuid: String,
}

/// Why a seat's pane is not proven.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A read did not answer; which one.
    Unread(String),
    /// The roster no longer holds that seat in that slot.
    Replaced,
    /// The session is not the one the chat is bound to.
    SessionReplaced,
    /// No pane carries the seat's slot.
    NoPane,
    /// More than one pane carries it.
    Ambiguous,
    /// The slot's pane carries another agent.
    OtherAgent,
    /// The pane's process is gone.
    Dead,
    /// The server does not list the pane in the session.
    NotMember,
    /// `ae app` runs outside tmux, so no client shows it.
    NoTmux,
    /// No tmux client has the app's pane as its active pane.
    NoViewer,
    /// Several clients showing the app were last active in the same second.
    Tied,
    /// The session is stopped; its name.
    Stopped(String),
    /// The session is not provably on the tmux server the app runs on; its name.
    Elsewhere(String),
    /// The seat the key meant is not known: the whole phrase.
    NoSeat(String),
}

impl Refusal {
    /// The line the chat prints for `name`.
    #[must_use]
    pub fn line(&self, name: &str) -> String {
        let why = match self {
            Self::Unread(what) => format!("{what} did not answer"),
            Self::Replaced => "the roster no longer seats it in that slot".to_owned(),
            Self::SessionReplaced => "this session was replaced".to_owned(),
            Self::NoPane => "no pane carries its slot".to_owned(),
            Self::Ambiguous => "more than one pane carries its slot".to_owned(),
            Self::OtherAgent => "its pane carries another agent".to_owned(),
            Self::Dead => "its pane is dead".to_owned(),
            Self::NotMember => "its pane is not in this session".to_owned(),
            Self::NoTmux => "ae app is not running inside tmux".to_owned(),
            Self::NoViewer => "no tmux client is showing this app".to_owned(),
            Self::Tied => "two clients showing this app were active in the same second".to_owned(),
            Self::Stopped(session) => format!("{session} is stopped; ae {session} resumes it"),
            Self::Elsewhere(session) => format!(
                "{session} is not on the tmux server this app runs on, or ae cannot prove it is; \
                 run ae {session}"
            ),
            Self::NoSeat(why) => why.clone(),
        };
        format!("refused: /open {name}: {why}; nothing selected")
    }
}

/// The facts the proof reads, each `None` when its read did not answer.
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    /// The seat as the chat last read it.
    pub seat: &'a SeatRef,
    /// The roster read now.
    pub roster: &'a [RosterEntry],
    /// The session uuid the chat is bound to.
    pub bound_uuid: &'a str,
    /// The session's `$<n>` id.
    pub session_id: Option<&'a str>,
    /// The session's `@ae_session_uuid`.
    pub uuid_stamp: Option<&'a str>,
    /// The session's panes by slot stamp.
    pub slots: Option<&'a [ObservedSlot]>,
    /// The session's panes by window.
    pub panes: Option<&'a [WindowPane]>,
    /// The server's pane memberships.
    pub members: Option<&'a [PickerPane]>,
}

/// The seat's pane, or why not.
///
/// # Errors
///
/// [`Refusal`], by name.
pub fn target(facts: &Facts<'_>) -> Result<Target, Refusal> {
    let seat = facts.seat;
    let seated = facts.roster.iter().find(|entry| entry.slot == seat.slot);
    if seated.is_none_or(|entry| entry.name != seat.name) {
        return Err(Refusal::Replaced);
    }
    let session_id = facts
        .session_id
        .filter(|_| canonical(facts.bound_uuid) && facts.uuid_stamp == Some(facts.bound_uuid))
        .ok_or(Refusal::SessionReplaced)?;
    let slots = facts
        .slots
        .ok_or_else(|| Refusal::Unread("the pane slots".to_owned()))?;
    let pane = match slots
        .iter()
        .filter(|row| row.slot == seat.slot)
        .collect::<Vec<_>>()[..]
    {
        [] => return Err(Refusal::NoPane),
        [row] if row.agent != seat.name => return Err(Refusal::OtherAgent),
        [row] => &row.pane,
        _ => return Err(Refusal::Ambiguous),
    };
    let panes = facts
        .panes
        .ok_or_else(|| Refusal::Unread("the session's panes".to_owned()))?;
    match panes.iter().find(|row| row.pane_id == *pane) {
        None => return Err(Refusal::NoPane),
        Some(row) if row.dead => return Err(Refusal::Dead),
        Some(_) => {}
    }
    let members = facts
        .members
        .ok_or_else(|| Refusal::Unread("the server's panes".to_owned()))?;
    if !members
        .iter()
        .any(|row| row.session_id == session_id && row.pane == *pane)
    {
        return Err(Refusal::NotMember);
    }
    Ok(Target {
        seat: seat.clone(),
        session_id: session_id.to_owned(),
        pane: pane.clone(),
        uuid: facts.bound_uuid.to_owned(),
    })
}

/// Whether `uuid` is a canonical session uuid, never an empty one.
fn canonical(uuid: &str) -> bool {
    !uuid.is_empty() && crate::archive::canonical_uuid(uuid) == uuid
}

/// Whether `text` is `sigil` followed by one or more ASCII digits.
fn numbered(text: &str, sigil: char) -> bool {
    text.strip_prefix(sigil)
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// The one tmux list that selects `target` inside its session; `None` when a
/// field fails its grammar.
#[must_use]
pub fn select_args(server: &ServerId, target: &Target) -> Option<Vec<String>> {
    select_args_via(server, target, None)
}

/// [`select_args`], handing `client` to the target's session first when one is
/// named: ONE `if-shell` whose guard holds before anything moves, so a pane
/// that changed since the proof moves no client. `None` for a client name that
/// fails its grammar.
#[must_use]
pub fn select_args_via(
    server: &ServerId,
    target: &Target,
    client: Option<&str>,
) -> Option<Vec<String>> {
    let Target {
        seat,
        session_id,
        pane,
        uuid,
    } = target;
    let proven = numbered(pane, '%')
        && numbered(session_id, '$')
        && crate::requests::is_slot(&seat.slot)
        && crate::config::is_agent_name(&seat.name)
        && canonical(uuid)
        && client.is_none_or(client_name);
    if !proven {
        return None;
    }
    // Every conjunction binary and nested: tmux 3.4 reads two operands only.
    let guard = format!(
        "#{{&&:#{{==:#{{session_id}},{session_id}}},#{{&&:#{{==:#{{@ae_session_uuid}},{uuid}}},\
         #{{&&:#{{==:#{{@ae_slot}},{slot}}},#{{&&:#{{==:#{{@ae_agent}},{name}}},\
         #{{==:#{{pane_dead}},0}}}}}}}}}}",
        slot = seat.slot,
        name = seat.name,
    );
    let switch = client.map_or_else(String::new, |client| {
        format!(
            "{} ; ",
            crate::tmux::switch_client_id_command(client, session_id)
        )
    });
    let mut args = crate::tmux::server_args(server);
    args.extend([
        "if-shell".to_owned(),
        "-F".to_owned(),
        "-t".to_owned(),
        pane.clone(),
        guard,
        format!(
            "{switch}select-window -t {pane} ; select-pane -t {pane} ; display-message -p {SELECTED}"
        ),
        format!("display-message -p {MOVED}"),
    ]);
    Some(args)
}

/// Whether `name` is a tty path or a control client's name: the characters
/// tmux gives one, none of which a tmux command word treats as syntax.
fn client_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
}

/// What an open moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Move {
    /// The target is the app's own session: tmux moves the session, whoever
    /// views it, so no client is named.
    Here,
    /// Hand this one client to the target's session.
    Client(String),
}

/// The move for an app whose pane is `app_pane`, opening a seat of `session`.
///
/// Only a client whose ACTIVE pane is the app can have pressed the key. None
/// refuses; all of them in the target session need no choice; otherwise the
/// one with the newest input (`None` the oldest) is the one that pressed it,
/// and a tie for newest names nobody.
///
/// # Errors
///
/// [`Refusal::NoViewer`] or [`Refusal::Tied`].
pub fn mover(clients: &[ObservedClient], app_pane: &str, session: &str) -> Result<Move, Refusal> {
    let viewers: Vec<&ObservedClient> = clients
        .iter()
        .filter(|client| client.pane == app_pane)
        .collect();
    if viewers.is_empty() {
        return Err(Refusal::NoViewer);
    }
    if viewers.iter().all(|client| client.session == session) {
        return Ok(Move::Here);
    }
    let newest = viewers.iter().map(|client| client.activity).max().flatten();
    let mut top = viewers.iter().filter(|client| client.activity == newest);
    match (top.next(), top.next()) {
        (Some(only), None) if only.session == session => Ok(Move::Here),
        (Some(only), None) => Ok(Move::Client(only.name.clone())),
        _ => Err(Refusal::Tied),
    }
}

/// What the select prints when the guard held, and when it did not.
const SELECTED: &str = "ae-open:selected";
const MOVED: &str = "ae-open:moved";

/// The chat's line for a finished select of `name`.
#[must_use]
pub fn outcome(succeeded: bool, stdout: &str, name: &str) -> String {
    match (succeeded, stdout.trim_end()) {
        (true, SELECTED) => format!("opened {name} - prefix h returns"),
        (true, MOVED) => {
            format!("refused: /open {name}: its pane changed after the read; nothing selected")
        }
        _ => format!("uncertain: /open {name}: tmux did not confirm the select; check the session"),
    }
}

/// Whether a finished select took: tmux ran it and the guard held.
#[must_use]
pub fn took(succeeded: bool, stdout: &str) -> bool {
    succeeded && stdout.trim_end() == SELECTED
}

/// [`outcome`] for a select that handed a client to `session`.
#[must_use]
pub fn outcome_moved(succeeded: bool, stdout: &str, name: &str, session: &str) -> String {
    match (succeeded, stdout.trim_end()) {
        (true, SELECTED) => {
            format!("opened {name} in {session} - prefix h opens its chat, prefix L goes back")
        }
        _ => outcome(succeeded, stdout, name),
    }
}

#[cfg(test)]
mod tests {
    use super::took;

    /// Only a select tmux ran whose guard held took; a refused guard, a failed
    /// run and unknown words did not.
    #[test]
    fn only_a_guarded_select_that_ran_took() {
        assert!(took(true, "ae-open:selected\n"));
        assert!(!took(false, "ae-open:selected\n"));
        assert!(!took(true, "ae-open:moved\n"));
        assert!(!took(true, ""));
    }
}
