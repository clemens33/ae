//! Option and identity writes: the watchdog's status publication, session and pane options, and the new-session identity.

use super::{
    FIELD_SEPARATOR, ServerId, ServerIdentity, SessionIdentity, is_decimal, pane_id_is_valid,
    server_args, session_target,
};

// ---------------------------------------------------------------------------
// The watchdog's tmux WRITES — status publication and the transient alert.
// ---------------------------------------------------------------------------

/// The session-scoped user option carrying the watchdog bar, `[watch <glyph>
/// <active>/<total>]`.
pub const WATCHDOG_STATUS_OPTION: &str = "@ae_watchdog_status";

/// How long a transient watchdog alert stays on screen, in milliseconds — the
/// frozen `display-message -d 10000`.
pub(super) const DISPLAY_MESSAGE_MS: &str = "10000";

/// Which option table a name lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionScope {
    /// A session option: `set-option -t <session-id> …`.
    Session,
    /// A window option: `set-option -w -t <window-id> …`.
    Window,
    /// A PANE option: `set-option -p -t <pane-id> …`.
    Pane,
}

impl OptionScope {
    /// The flag tmux needs for this table, if any.
    const fn flag(self) -> Option<&'static str> {
        match self {
            Self::Session => None,
            Self::Window => Some("-w"),
            Self::Pane => Some("-p"),
        }
    }
}

/// Escape text that is about to enter a tmux FORMAT context — `#` then `%`.
#[must_use]
pub fn format_literal(text: &str) -> String {
    text.replace('#', "##").replace('%', "%%")
}

/// Set one user option on `target`, which must name the selected table.
#[must_use]
pub fn set_option_args(
    server: &ServerId,
    scope: OptionScope,
    target: &str,
    name: &str,
    value: &str,
) -> Vec<String> {
    let mut args = server_args(server);
    args.push("set-option".to_owned());
    if let Some(flag) = scope.flag() {
        args.push(flag.to_owned());
    }
    args.push("-t".to_owned());
    args.push(match scope {
        OptionScope::Session => format!("{}:", session_target(target)),
        OptionScope::Window | OptionScope::Pane => target.to_owned(),
    });
    args.extend([name, value].map(ToOwned::to_owned));
    args
}

/// ONE server-side operation that sets a session option to `value` only while
/// the session is still the PROVEN incarnation and the option is vacant.
///
/// `if-shell -F` is evaluated by the tmux server, in its own command queue, and
/// its `set-option` branch runs in the SAME queued command: no replacement can
/// interleave between the checks and the write. The guard reads only facts the
/// server itself owns:
///
/// * `#{pid}` and `#{start_time}` — the server incarnation;
/// * `#{session_id}` and `#{session_created}` — the session incarnation (the id
///   alone is reused once a server empties, and the creation instant is whole
///   seconds);
/// * `#{<name>}` — the option is still unset or empty.
///
/// Every component is ae-validated (`$<n>`, decimal epochs, a canonical value),
/// so the guard and the branch need no quoting. Conjunctions stay binary and
/// nested: tmux 3.4 reads only the first two operands and drops a third.
#[must_use]
pub fn guarded_session_option_args(
    server: &ServerId,
    expected_server: &ServerIdentity,
    expected_session: &SessionIdentity,
    name: &str,
    value: &str,
) -> Vec<String> {
    let target = format!("{}:", expected_session.id);
    let guard = format!(
        "#{{&&:#{{&&:#{{==:#{{pid}},{pid}}},#{{==:#{{start_time}},{start}}}}},#{{&&:#{{&&:#{{==:#{{session_id}},{id}}},#{{==:#{{session_created}},{created}}}}},#{{==:#{{{name}}},}}}}}}",
        pid = expected_server.pid,
        start = expected_server.start,
        id = expected_session.id,
        created = expected_session.created,
    );
    let mut args = server_args(server);
    args.extend(["if-shell", "-F", "-t"].map(ToOwned::to_owned));
    args.push(expected_session.id.clone());
    args.push(guard);
    args.push(format!("set-option -t {target} {name} {value}"));
    args
}

/// The `-P -F` format `new-session` itself prints: the identity of the session
/// it JUST created, in the SAME command that created it.
///
/// This is what makes the launch's proof atomic: there is no later capture to
/// interleave a replacement into. The server pair (`#{pid}`, `#{start_time}`)
/// and the session pair (`#{session_id}`, `#{session_created}`) are read by the
/// server at creation, and the pane id is the handle ae keeps.
pub const NEW_SESSION_IDENTITY_FORMAT: &str =
    "#{session_id} | #{session_created} | #{pid} | #{start_time} | #{pane_id}";

/// What one successful `new-session -P -F` printed: the full proven identity
/// and the created pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    /// The server incarnation at creation.
    pub server: ServerIdentity,
    /// The session incarnation at creation.
    pub session: SessionIdentity,
    /// The first pane's `%<n>`.
    pub pane: String,
}

/// The arguments creating a session and printing its full identity at once.
#[must_use]
pub fn new_session_identity_args(name: &str, work_dir: &str) -> Vec<String> {
    [
        "new-session",
        "-d",
        "-s",
        name,
        "-c",
        work_dir,
        "-P",
        "-F",
        NEW_SESSION_IDENTITY_FORMAT,
    ]
    .map(ToOwned::to_owned)
    .to_vec()
}

/// Parse one `new-session -P -F` line into the created identity, or `None` for
/// a failed run, a wrong field count, a non-decimal epoch or a malformed pane.
#[must_use]
pub fn interpret_new_session(succeeded: bool, stdout: &str) -> Option<NewSession> {
    if !succeeded {
        return None;
    }
    let line = stdout.lines().next()?;
    let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
    let [id, created, pid, start, pane] = fields.as_slice() else {
        return None;
    };
    if id.is_empty()
        || !is_decimal(created)
        || !is_decimal(pid)
        || !is_decimal(start)
        || !pane_id_is_valid(pane)
    {
        return None;
    }
    Some(NewSession {
        server: ServerIdentity {
            pid: (*pid).to_owned(),
            start: (*start).to_owned(),
        },
        session: SessionIdentity {
            id: (*id).to_owned(),
            created: (*created).to_owned(),
        },
        pane: (*pane).to_owned(),
    })
}

/// `#{session_id} | #{session_created}` for ONE pane's session — the read that
/// binds a launch's identity to the pane ae CREATED, never to the mutable name.
pub const PANE_SESSION_IDENTITY_FORMAT: &str = "#{session_id} | #{session_created}";

/// The arguments asking which session one exact pane belongs to, and when that
/// session was created.
#[must_use]
pub fn pane_session_identity_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        [
            "display-message",
            "-p",
            "-t",
            pane,
            PANE_SESSION_IDENTITY_FORMAT,
        ]
        .map(ToOwned::to_owned),
    );
    args
}

/// The identity one pane's session reports, or `None` when the read failed,
/// the line is malformed or the creation instant is not a decimal epoch.
#[must_use]
pub fn interpret_pane_session_identity(succeeded: bool, stdout: &str) -> Option<SessionIdentity> {
    if !succeeded {
        return None;
    }
    let line = stdout.lines().next()?;
    let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
    let [id, created] = fields.as_slice() else {
        return None;
    };
    if id.is_empty() || !is_decimal(created) {
        return None;
    }
    Some(SessionIdentity {
        id: (*id).to_owned(),
        created: (*created).to_owned(),
    })
}

/// One `set-option` in a batched tmux command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OptionWrite {
    scope: OptionScope,
    target: String,
    name: String,
    value: String,
}

impl OptionWrite {
    /// A write to one exact option table and target.
    pub(crate) fn new(scope: OptionScope, target: &str, name: &str, value: &str) -> Self {
        Self {
            scope,
            target: target.to_owned(),
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }

    /// Which option this write sets — the fact a caller writing into ANOTHER
    /// session's table has to be held to, read directly rather than recovered
    /// from its argv. Only the pins on that boundary ask.
    #[cfg(test)]
    pub(crate) fn option_name(&self) -> &str {
        &self.name
    }
}

/// Batch several option writes into one tmux invocation. `;` is the argv form
/// of the shell spelling `\;`; no shell sits between this vector and tmux.
#[must_use]
pub(crate) fn set_options_args(server: &ServerId, writes: &[OptionWrite]) -> Vec<String> {
    let mut args = server_args(server);
    for (index, write) in writes.iter().enumerate() {
        if index > 0 {
            args.push(";".to_owned());
        }
        args.extend(set_option_words(write));
    }
    args
}

/// One `set-option` command of a queued invocation, without the server words.
pub(super) fn set_option_words(write: &OptionWrite) -> Vec<String> {
    let mut words = vec!["set-option".to_owned()];
    if let Some(flag) = write.scope.flag() {
        words.push(flag.to_owned());
    }
    words.push("-t".to_owned());
    words.push(option_target(write.scope, &write.target));
    words.extend([write.name.clone(), write.value.clone()]);
    words
}

/// One `set-option -u` command of a queued invocation, without the server words.
pub(super) fn unset_option_words(scope: OptionScope, target: &str, name: &str) -> Vec<String> {
    let mut words = vec!["set-option".to_owned()];
    if let Some(flag) = scope.flag() {
        words.push(flag.to_owned());
    }
    words.extend(["-u", "-t"].map(ToOwned::to_owned));
    words.push(option_target(scope, target));
    words.push(name.to_owned());
    words
}

/// The `-t` an option command names: an exact session table, or a window or
/// pane id as given.
pub(super) fn option_target(scope: OptionScope, target: &str) -> String {
    match scope {
        OptionScope::Session => format!("{}:", session_target(target)),
        OptionScope::Window | OptionScope::Pane => target.to_owned(),
    }
}
