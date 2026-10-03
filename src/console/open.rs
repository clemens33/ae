//! `/open <agent>`: the proof that a seat's pane is still the one the chat
//! names, and the one tmux list that selects it inside the chat's own session.
//!
//! PURE — the reads arrive as arguments. The proof is separate from the act so
//! a reader with no tmux client of its own can reuse it.

use crate::console::needs::SeatRef;
use crate::inventory::ServerId;
use crate::meta::RosterEntry;
use crate::tmux::{ObservedSlot, PickerPane, WindowPane};

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
}

impl Refusal {
    /// The line the chat prints for `name`.
    #[must_use]
    pub fn line(&self, name: &str) -> String {
        let _ = (self, name);
        String::new()
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
    let _ = facts;
    Err(Refusal::Unread("not built".to_owned()))
}

/// The one tmux list that selects `target` inside its session; `None` when a
/// field fails its grammar.
#[must_use]
pub fn select_args(server: &ServerId, target: &Target) -> Option<Vec<String>> {
    let _ = (server, target);
    None
}

/// The chat's line for a finished select of `name`.
#[must_use]
pub fn outcome(succeeded: bool, stdout: &str, name: &str) -> String {
    let _ = (succeeded, stdout, name);
    String::new()
}
