//! `!<command>`: the human's own line, run outside the session in its work
//! dir — a tmux popup on the client showing the composer, or a detached
//! `/bin/sh` when there is no tmux. ae authors no script text: the line is the
//! human's, handed over as one argument, and it is never recorded anywhere.
//!
//! PURE — the reads arrive as arguments, and the one place that acts on what
//! this module builds is `console::term::shell`.

use crate::console::open::{self, Refusal};
use crate::inventory::ServerId;
use crate::tmux::ObservedClient;

/// Where a `!` line runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place<'a> {
    /// In a popup on the tmux client whose active pane is `pane`.
    Tmux { server: &'a ServerId, pane: &'a str },
    /// No tmux pane: a detached `/bin/sh`.
    Outside,
}

/// Whether `line` is one line to run: not blank, and no control character
/// but a tab, so nothing in it can end the line or act on a terminal.
fn one_line(line: &str) -> bool {
    !line.trim().is_empty() && !line.chars().any(|ch| ch.is_control() && ch != '\t')
}

/// The tmux argv opening `line` in a popup on `client`, started in `dir`, on
/// `server`: `None` when a fact fails its grammar. tmux expands no format in
/// the popup's command, so the line goes over as it is, with one space after a
/// trailing `;`, which tmux's own parse would otherwise take as a separator.
#[must_use]
pub fn popup_args(server: &ServerId, client: &str, dir: &str, line: &str) -> Option<Vec<String>> {
    let named = open::client_name(client) && !client.starts_with('-');
    let at = dir.starts_with('/') && !dir.ends_with(';') && !dir.chars().any(char::is_control);
    if !named || !at || !one_line(line) {
        return None;
    }
    let mut args = crate::tmux::server_args(server);
    args.extend(["display-popup", "-EE", "-c", client, "-d", dir, "--"].map(str::to_owned));
    args.push(if line.ends_with(';') {
        format!("{line} ")
    } else {
        line.to_owned()
    });
    Some(args)
}

/// The client a popup opens on: the one whose active pane is `pane` with the
/// newest input — the one that pressed Enter.
///
/// # Errors
///
/// No client shows the pane, or two tie for newest.
pub fn popup_client(clients: &[ObservedClient], pane: &str) -> Result<String, String> {
    open::copy_client(clients, pane).map_err(|refusal| match refusal {
        Refusal::Tied => "two clients showing this pane were active in the same second".to_owned(),
        _ => "no tmux client is showing this pane".to_owned(),
    })
}

/// The arguments `/bin/sh` runs one line with: `-c` and the line. Sealed, so
/// only a line that passed the one-line rule can reach a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellArgv(Vec<String>);

impl ShellArgv {
    /// `-c <line>`, or `None` when `line` is not one line to run.
    #[must_use]
    pub fn of(line: &str) -> Option<Self> {
        one_line(line).then(|| Self(vec!["-c".to_owned(), line.to_owned()]))
    }

    /// The arguments, after the program.
    #[must_use]
    pub fn as_args(&self) -> &[String] {
        &self.0
    }
}

/// The work dir a session's `meta` bytes record.
///
/// # Errors
///
/// It records none, or one that is not absolute.
pub fn work_dir(meta: &[u8]) -> Result<String, String> {
    let dir = crate::meta::first_value(meta, "work_dir").map(String::from_utf8_lossy);
    match dir.as_deref() {
        None | Some("") => Err("the session records no work dir".to_owned()),
        Some(dir) if !dir.starts_with('/') => Err(format!("its work dir {dir} is not absolute")),
        Some(dir) => Ok(dir.to_owned()),
    }
}

/// What a `!` line that started says: a popup, or a detached shell.
#[must_use]
pub fn said(line: &str, popup: bool) -> String {
    if popup {
        format!("! {line} - popup started")
    } else {
        format!("! {line} - started")
    }
}

/// What a `!` line that did not start says, and `why`.
#[must_use]
pub fn refused(line: &str, why: &str) -> String {
    format!("refused: ! {line}: {why}")
}
