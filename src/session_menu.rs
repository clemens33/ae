//! `_session-menu` — the forward chain behind a right-click on one session's
//! status range.
//!
//! Three steps, each its own process, each carrying the SAME captured facts as
//! literal arguments:
//!
//! 1. tmux draws the centred context menu itself from the status binding. That
//!    draw writes nothing.
//! 2. `confirm` proves the captured facts still describe the live world, then
//!    draws a second centred menu on the SAME client: Cancel first, the exact
//!    consequence in the middle, the destructive row last.
//! 3. `apply` re-proves everything, then hands the operation to the EXISTING
//!    detached lifecycle owner, which proves it once more under that session's
//!    lifecycle lock before anything is killed.
//!
//! No step invents identity. A menu row carries what the click saw; every
//! later step either matches it against the live server or refuses on the
//! client that asked.

use std::io::{self, Write};
use std::path::Path;

use crate::inventory::ServerId;

/// The context menu's destructive action. `end` follows in its own phase.
pub const STOP: &str = "stop";

/// The settings menu's recoverable orchestrator pause.
pub const PAUSE_ORCHESTRATOR: &str = "pause-orchestrator";

/// The chain's two steps, as the row that queues each one spells it.
pub const CONFIRM: &str = "confirm";
pub const APPLY: &str = "apply";

/// The context-menu row that starts the stop chain. ASCII, because the row is
/// drawn from a server-global binding that no session's look reaches.
pub const STOP_ROW_LABEL: &str = "Stop session...";

/// How long a built confirmation stays answerable, in seconds.
///
/// The human is being asked a question about a live session; an answer given
/// long after the question was posed is about a world that may have moved. The
/// deadline is stamped once, when the confirmation is BUILT, and carried
/// unchanged through the apply and the detached supervisor.
pub const CONFIRM_WINDOW_SECS: i64 = 120;

/// The usage line, for an argv this module cannot read.
pub const USAGE: &str = "Usage: _session-menu <confirm|apply> --action <stop|pause-orchestrator> --client <name> --client-pid <pid> --session <name> --session-id <$id> --pane <%id> --server-pid <pid> --server-start <epoch> [--uuid <uuid> --deadline <epoch>]";

/// The facts one click captured, proven against this crate's grammars.
///
/// Every field is an IDENTITY, not a hint: the pair of server fields says
/// which tmux server the click happened on, the client pair says which
/// attachment asked, and the session pair says which session was clicked. A
/// later step that cannot match all three refuses rather than choosing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// The action word: ordinary Stop or orchestrator Pause.
    pub action: String,
    /// The invoking client's tmux name, normally its tty path.
    pub client: String,
    /// That client's process, which separates one attachment from the next
    /// attachment on the same tty.
    pub client_pid: String,
    /// The clicked session's name.
    pub session: String,
    /// The clicked session's tmux id, which a rename does not change.
    pub session_id: String,
    /// The pane the click resolved to — the menu's command context.
    pub pane: String,
    /// The tmux server's process.
    pub server_pid: String,
    /// The epoch second that server started.
    pub server_start: String,
    /// ae's own session identity, snapshotted when the confirmation is built.
    pub uuid: String,
    /// The epoch second after which the confirmation is stale.
    pub deadline: i64,
}

/// What a captured argv failed to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The argv is not this grammar at all.
    Usage(String),
    /// The argv parsed, but a field is not the shape ae admits.
    Field(String),
}

impl Refusal {
    /// The message a human reads.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Usage(text) | Self::Field(text) => text,
        }
    }

    /// The exit code this refusal carries.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Usage(_) => crate::entry::EXIT_USAGE,
            Self::Field(_) => crate::entry::EXIT_FAILED,
        }
    }
}

/// Which step of the chain an argv asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Prove the capture, then ask the human.
    Confirm,
    /// Prove the capture and the human's answer, then hand it over.
    Apply,
}

/// A client name is a tmux target word and reaches a shell, so it is an
/// allowlist: the tty paths tmux mints, and nothing that could become syntax.
fn client_name_is_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._/:+-".contains(&b))
}

/// Read one `--flag <value>` pair into `slot`, refusing a repeat.
fn take(slot: &mut Option<String>, flag: &str, value: Option<&String>) -> Result<(), Refusal> {
    if slot.is_some() {
        return Err(Refusal::Usage(format!("{flag} may be given only once")));
    }
    let Some(value) = value else {
        return Err(Refusal::Usage(format!("{flag} requires a value")));
    };
    *slot = Some(value.clone());
    Ok(())
}

/// A positive decimal, or the refusal naming the flag that was not one.
fn positive(flag: &str, value: &str) -> Result<i64, Refusal> {
    value
        .parse::<i64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| Refusal::Field(format!("{flag} is not a positive number: {value:?}")))
}

/// Parse and PROVE one step's argv.
///
/// # Errors
/// Returns the refusal describing the first word or field that is not this
/// grammar.
#[allow(
    clippy::too_many_lines,
    reason = "one ordered grammar: every captured field is read and proven in one place"
)]
pub fn parse(tail: &[String]) -> Result<(Step, Captured), Refusal> {
    let Some((step, rest)) = tail.split_first() else {
        return Err(Refusal::Usage(USAGE.to_owned()));
    };
    let step = match step.as_str() {
        CONFIRM => Step::Confirm,
        APPLY => Step::Apply,
        other => {
            return Err(Refusal::Usage(format!("unknown step {other:?}. {USAGE}")));
        }
    };
    let mut action = None;
    let mut client = None;
    let mut client_pid = None;
    let mut session = None;
    let mut session_id = None;
    let mut pane = None;
    let mut server_pid = None;
    let mut server_start = None;
    let mut uuid = None;
    let mut deadline = None;
    let mut index = 0;
    while index < rest.len() {
        let flag = rest[index].as_str();
        let value = rest.get(index + 1);
        let slot = match flag {
            "--action" => &mut action,
            "--client" => &mut client,
            "--client-pid" => &mut client_pid,
            "--session" => &mut session,
            "--session-id" => &mut session_id,
            "--pane" => &mut pane,
            "--server-pid" => &mut server_pid,
            "--server-start" => &mut server_start,
            "--uuid" => &mut uuid,
            "--deadline" => &mut deadline,
            other => {
                return Err(Refusal::Usage(format!("unknown flag {other:?}. {USAGE}")));
            }
        };
        take(slot, flag, value)?;
        index += 2;
    }
    let missing = |flag: &str| Refusal::Usage(format!("{flag} is required. {USAGE}"));
    let action = action.ok_or_else(|| missing("--action"))?;
    if action != STOP && action != PAUSE_ORCHESTRATOR {
        return Err(Refusal::Field(format!(
            "unsupported action {action:?} — this menu offers {STOP:?} or {PAUSE_ORCHESTRATOR:?}"
        )));
    }
    let client = client.ok_or_else(|| missing("--client"))?;
    if !client_name_is_valid(&client) {
        return Err(Refusal::Field(format!(
            "{client:?} is not a tmux client name ae will address"
        )));
    }
    let client_pid = client_pid.ok_or_else(|| missing("--client-pid"))?;
    positive("--client-pid", &client_pid)?;
    let session = session.ok_or_else(|| missing("--session"))?;
    if !crate::lifecycle::name_is_valid(&session) {
        return Err(Refusal::Field(format!(
            "{session:?} is not an ae session name"
        )));
    }
    let session_id = session_id.ok_or_else(|| missing("--session-id"))?;
    if !crate::tmux::session_id_is_valid(&session_id) {
        return Err(Refusal::Field(format!(
            "{session_id:?} is not a tmux session id"
        )));
    }
    let pane = pane.ok_or_else(|| missing("--pane"))?;
    if !crate::tmux::pane_id_is_valid(&pane) {
        return Err(Refusal::Field(format!("{pane:?} is not a tmux pane id")));
    }
    let server_pid = server_pid.ok_or_else(|| missing("--server-pid"))?;
    positive("--server-pid", &server_pid)?;
    let server_start = server_start.ok_or_else(|| missing("--server-start"))?;
    positive("--server-start", &server_start)?;
    let (uuid, deadline) = match step {
        Step::Confirm => {
            if uuid.is_some() || deadline.is_some() {
                return Err(Refusal::Usage(
                    "confirm mints --uuid and --deadline; it does not take them".to_owned(),
                ));
            }
            (String::new(), 0)
        }
        Step::Apply => {
            let uuid = uuid.ok_or_else(|| missing("--uuid"))?;
            let canonical = crate::archive::canonical_uuid(&uuid);
            if canonical.is_empty() {
                return Err(Refusal::Field(format!("{uuid:?} is not a session uuid")));
            }
            let deadline = deadline.ok_or_else(|| missing("--deadline"))?;
            (canonical, positive("--deadline", &deadline)?)
        }
    };
    Ok((
        step,
        Captured {
            action,
            client,
            client_pid,
            session,
            session_id,
            pane,
            server_pid,
            server_start,
            uuid,
            deadline,
        },
    ))
}

impl Captured {
    /// The argv the confirmation's destructive row runs, with the identity
    /// this step proved and the deadline it stamped.
    #[must_use]
    pub fn apply_argv(&self, core: &Path, uuid: &str, deadline: i64) -> Vec<String> {
        vec![
            core.display().to_string(),
            crate::cli::SESSION_MENU.to_owned(),
            APPLY.to_owned(),
            "--action".to_owned(),
            self.action.clone(),
            "--client".to_owned(),
            self.client.clone(),
            "--client-pid".to_owned(),
            self.client_pid.clone(),
            "--session".to_owned(),
            self.session.clone(),
            "--session-id".to_owned(),
            self.session_id.clone(),
            "--pane".to_owned(),
            self.pane.clone(),
            "--server-pid".to_owned(),
            self.server_pid.clone(),
            "--server-start".to_owned(),
            self.server_start.clone(),
            "--uuid".to_owned(),
            uuid.to_owned(),
            "--deadline".to_owned(),
            deadline.to_string(),
        ]
    }
}

/// The room one menu needs on a client, in columns and rows.
///
/// tmux does NOT refuse a row it cannot fit: `menu_add_item` trims the text to
/// the client's width less its borders, and less the key column when it keeps
/// one, before the menu is ever prepared. A drawn confirmation is therefore no
/// proof that the consequence on it is legible — so ae measures the rows it is
/// about to ask for and refuses the question rather than asking half of it.
///
/// Source: <https://raw.githubusercontent.com/tmux/tmux/3.4/menu.c> lines
/// 84-109 (the trim) and 449-450 (the fit).
#[must_use]
pub fn menu_budget(menu: &crate::tmux::Menu) -> (usize, usize) {
    // 4: the two border columns and the one space of padding each side.
    const BORDERS: usize = 4;
    // 3: the space and the two brackets tmux puts around a row's key.
    const KEY_BRACKETS: usize = 3;
    let width = menu
        .items
        .iter()
        .map(|item| {
            let key = if item.key.is_empty() {
                0
            } else {
                item.key.chars().count() + KEY_BRACKETS
            };
            item.label.chars().count() + BORDERS + key
        })
        .chain(std::iter::once(menu.title.chars().count() + BORDERS))
        .max()
        .unwrap_or(BORDERS);
    // Two border rows, and one status line ae must not draw over.
    (width, menu.items.len() + 3)
}

/// The rows of the stop confirmation, Cancel first and destructive last.
#[must_use]
pub fn stop_confirmation(session: &str, apply: &str) -> crate::tmux::Menu {
    crate::tmux::Menu {
        title: format!("Stop session '{session}'?"),
        title_style: String::new(),
        items: vec![
            crate::tmux::MenuItem {
                label: "Cancel".to_owned(),
                key: "c".to_owned(),
                // Cancel QUEUES NOTHING. Dismissing the menu and choosing this
                // row must be the same act.
                action: crate::tmux::MenuAction::Run(String::new()),
            },
            crate::tmux::MenuItem {
                label: String::new(),
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: format!("Stops the ae session '{session}' and its agents."),
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: "Its state, worktree and conversations are PRESERVED.".to_owned(),
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: "Agents mid-turn are interrupted; resume it with 'ae'.".to_owned(),
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: String::new(),
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: format!("Stop '{session}' now"),
                // A key of its own, never the one Cancel or Flip answers to.
                key: "S".to_owned(),
                action: crate::tmux::MenuAction::Run(apply.to_owned()),
            },
        ],
    }
}

/// The state-preserving confirmation for the proven orchestrator role.
#[must_use]
pub fn pause_confirmation(session: &str, apply: &str) -> crate::tmux::Menu {
    let resume = if session == crate::orchestrator::ORCHESTRATOR_SESSION {
        "Resume it with 'ae orchestrator --no-attach'.".to_owned()
    } else {
        format!("Resume it with 'ae {session} --no-attach'.")
    };
    crate::tmux::Menu {
        title: format!("Pause orchestrator '{session}'?"),
        title_style: String::new(),
        items: vec![
            crate::tmux::MenuItem {
                label: "Cancel".to_owned(),
                key: "c".to_owned(),
                action: crate::tmux::MenuAction::Run(String::new()),
            },
            crate::tmux::MenuItem {
                label: String::new(),
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: "Stops the orchestrator session and its agents.".to_owned(),
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: "Its state, worktree and conversations are PRESERVED.".to_owned(),
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: resume,
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: String::new(),
                key: String::new(),
                action: crate::tmux::MenuAction::Disabled,
            },
            crate::tmux::MenuItem {
                label: format!("Pause '{session}' now"),
                key: "P".to_owned(),
                action: crate::tmux::MenuAction::Run(apply.to_owned()),
            },
        ],
    }
}

/// The invoking attachment, proven to be the one that clicked.
struct Clicker {
    server: ServerId,
    client: crate::tmux::MenuClient,
}

/// Say `text` on the captured client, and on stderr whatever tmux does.
///
/// The invoking human is looking at a menu, not at a `run-shell` job's stderr,
/// so the client is the primary channel — but the stream is still written, so
/// a lost client is not a silent failure.
fn report(server: Option<&ServerId>, captured: &Captured, text: &str, err: &mut impl Write) {
    if let Some(server) = server {
        let _ = crate::transport::display_client_message(server, &captured.client, text);
    }
    let _ = writeln!(err, "ae session menu: {text}");
}

/// Prove the SERVER the click happened on and the ATTACHMENT that made it.
///
/// This comes first in every step and touches nothing: until ae knows which
/// human is owed the answer, it cannot report a refusal, and until it knows
/// the server, no other captured fact means anything.
fn prove_clicker(captured: &Captured, err: &mut impl Write) -> Option<Clicker> {
    let Some(server) = crate::doors::caller_server() else {
        let _ = writeln!(
            err,
            "ae session menu: no calling tmux server, so ae cannot prove what was clicked."
        );
        return None;
    };
    let Some(identity) = crate::transport::observe_server_identity(&server) else {
        report(
            None,
            captured,
            "the tmux server did not answer with its identity; nothing was done.",
            err,
        );
        return None;
    };
    if identity.pid != captured.server_pid || identity.start != captured.server_start {
        report(
            None,
            captured,
            "this tmux server restarted since the menu was opened; nothing was done.",
            err,
        );
        return None;
    }
    let Some(client) = crate::transport::observe_menu_client(&server, &captured.client) else {
        let _ = writeln!(
            err,
            "ae session menu: client {:?} is no longer attached; nothing was done.",
            captured.client
        );
        return None;
    };
    if client.pid != captured.client_pid {
        let _ = writeln!(
            err,
            "ae session menu: client {:?} is a different attachment now; nothing was done.",
            captured.client
        );
        return None;
    }
    Some(Clicker { server, client })
}

/// Prove the TARGET is the session that was clicked and that ae owns it, and
/// return ae's own identity for its state directory.
fn prove_target(
    root: &Path,
    captured: &Captured,
    clicker: &Clicker,
    err: &mut impl Write,
) -> Option<String> {
    let server = &clicker.server;
    if !crate::lifecycle::name_is_usable(root, &captured.session) {
        report(
            Some(server),
            captured,
            &format!("'{}' is not an ae session ae owns.", captured.session),
            err,
        );
        return None;
    }
    let Some(live) = crate::lifecycle::live_id(server, &captured.session) else {
        report(
            Some(server),
            captured,
            &format!("'{}' is not running any more.", captured.session),
            err,
        );
        return None;
    };
    if live != captured.session_id {
        report(
            Some(server),
            captured,
            &format!(
                "'{}' is a different session now ({live} was {}); nothing was done.",
                captured.session, captured.session_id
            ),
            err,
        );
        return None;
    }
    let Some(owner) = crate::transport::observe_pane_owner(server, &captured.pane) else {
        report(
            Some(server),
            captured,
            "the clicked pane is gone; nothing was done.",
            err,
        );
        return None;
    };
    if owner.session != captured.session {
        report(
            Some(server),
            captured,
            "the clicked pane moved to another session; nothing was done.",
            err,
        );
        return None;
    }
    // ae'S OWN RECORD: a state directory replaced under the same name is a
    // different session, whatever tmux still calls it.
    let dir = crate::lifecycle::sessions_dir(root).join(&captured.session);
    let Ok(bytes) = crate::meta::read_bytes(&dir) else {
        report(
            Some(server),
            captured,
            &format!("ae cannot read the metadata of '{}'.", captured.session),
            err,
        );
        return None;
    };
    if captured.action == PAUSE_ORCHESTRATOR
        && crate::meta::meta_agent_role(&bytes) != crate::meta::MetaAgentRole::Role
    {
        report(
            Some(server),
            captured,
            &format!(
                "'{}' no longer proves the orchestrator role; nothing was done.",
                captured.session
            ),
            err,
        );
        return None;
    }
    let uuid = crate::archive::canonical_uuid(&crate::lifecycle::meta_value(&bytes, "session_id"));
    if uuid.is_empty() {
        report(
            Some(server),
            captured,
            &format!("'{}' records no session identity.", captured.session),
            err,
        );
        return None;
    }
    if !captured.uuid.is_empty() && captured.uuid != uuid {
        report(
            Some(server),
            captured,
            &format!(
                "the state of '{}' was replaced since the menu was opened; nothing was done.",
                captured.session
            ),
            err,
        );
        return None;
    }
    Some(uuid)
}

/// `_session-menu confirm …` — prove the click, then ask the human.
fn run_confirm(root: &Path, captured: &Captured, err: &mut impl Write) -> u8 {
    let Some(clicker) = prove_clicker(captured, err) else {
        return crate::entry::EXIT_FAILED;
    };
    let Some(uuid) = prove_target(root, captured, &clicker, err) else {
        return crate::entry::EXIT_FAILED;
    };
    let Some(core) = crate::shape::resolved_exe() else {
        report(
            Some(&clicker.server),
            captured,
            "ae cannot name its own executable, so it cannot offer the action.",
            err,
        );
        return crate::entry::EXIT_FAILED;
    };
    let deadline = crate::time::Timestamp::now().epoch() + CONFIRM_WINDOW_SECS;
    let apply = crate::tmux::menu_run_shell_command(&captured.apply_argv(&core, &uuid, deadline));
    let menu = if captured.action == PAUSE_ORCHESTRATOR {
        pause_confirmation(&captured.session, &apply)
    } else {
        stop_confirmation(&captured.session, &apply)
    };
    // THE WHOLE QUESTION OR NONE OF IT. tmux would trim the consequence and
    // still draw a destructive row; a question a human cannot read in full is
    // not a question ae is willing to ask.
    let (columns, rows) = menu_budget(&menu);
    if clicker.client.width < columns || clicker.client.height < rows {
        report(
            Some(&clicker.server),
            captured,
            &format!(
                "this terminal is {}x{}; saying what stopping '{}' does needs {columns}x{rows}.",
                clicker.client.width, clicker.client.height, captured.session
            ),
            err,
        );
        return crate::entry::EXIT_FAILED;
    }
    // The same capability split the status bindings were installed under: a
    // 3.4 menu is keyboard-driven, and passing `-M` to it fails the draw.
    let menu_mouse = match crate::transport::probe_tmux_version(&clicker.server) {
        crate::tmux::VersionProbe::Answered(found) => {
            crate::tmux_floor::Probe::Server(found).menu_mouse()
        }
        crate::tmux::VersionProbe::NoServer | crate::tmux::VersionProbe::Unreachable => false,
    };
    if !crate::transport::display_menu_centred(
        &clicker.server,
        &captured.client,
        &captured.pane,
        &menu,
        menu_mouse,
    ) {
        report(
            Some(&clicker.server),
            captured,
            "tmux refused to draw the confirmation; nothing was done.",
            err,
        );
        return crate::entry::EXIT_FAILED;
    }
    0
}

/// `_session-menu apply …` — the human answered; prove it all again, then hand
/// the operation to the lifecycle owner.
fn run_apply(
    root: &Path,
    captured: &Captured,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    // THE CLICKER FIRST, even for a refusal: an expired answer that reports
    // only to a `run-shell` job's stderr is an answer nobody ever sees. Proving
    // the attachment writes nothing, so the deadline still gates every effect.
    let Some(clicker) = prove_clicker(captured, err) else {
        return Ok(crate::entry::EXIT_FAILED);
    };
    let now = crate::time::Timestamp::now().epoch();
    if now > captured.deadline {
        report(
            Some(&clicker.server),
            captured,
            &format!(
                "this confirmation for '{}' expired; nothing was done — ask again.",
                captured.session
            ),
            err,
        );
        return Ok(crate::entry::EXIT_FAILED);
    }
    if now < captured.deadline - CONFIRM_WINDOW_SECS {
        report(
            Some(&clicker.server),
            captured,
            &format!(
                "this confirmation for '{}' is stamped in the future; nothing was done.",
                captured.session
            ),
            err,
        );
        return Ok(crate::entry::EXIT_FAILED);
    }
    let Some(uuid) = prove_target(root, captured, &clicker, err) else {
        return Ok(crate::entry::EXIT_FAILED);
    };
    let code = crate::lifecycle::stop_confirmed(
        root,
        &crate::lifecycle::StopExpectation {
            session_id: captured.session_id.clone(),
            uuid,
            server_pid: captured.server_pid.clone(),
            server_start: captured.server_start.clone(),
            deadline: captured.deadline,
            client: captured.client.clone(),
            client_pid: captured.client_pid.clone(),
            server: clicker.server.clone(),
            require_meta_agent: captured.action == PAUSE_ORCHESTRATOR,
        },
        &captured.session,
        out,
        err,
    )?;
    if code != 0 {
        // This process is a tmux job: its streams reach nobody. The human who
        // answered the question is owed the answer.
        report(
            Some(&clicker.server),
            captured,
            &format!("could not start the stop of '{}'.", captured.session),
            err,
        );
    }
    Ok(code)
}

/// `_session-menu <step> …` — the whole internal chain.
///
/// # Errors
/// Propagates a write failure on the caller's streams.
pub fn run(
    root: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let (step, captured) = match parse(tail) {
        Ok(parsed) => parsed,
        Err(refusal) => {
            writeln!(err, "ae session menu: {}", refusal.message())?;
            return Ok(refusal.code());
        }
    };
    match step {
        Step::Confirm => Ok(run_confirm(root, &captured, err)),
        Step::Apply => run_apply(root, &captured, out, err),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PAUSE_ORCHESTRATOR, Refusal, STOP, Step, menu_budget, parse, pause_confirmation,
        stop_confirmation,
    };

    fn argv(step: &str, extra: &[&str]) -> Vec<String> {
        let mut words = vec![
            step,
            "--action",
            "stop",
            "--client",
            "/dev/ttys004",
            "--client-pid",
            "4242",
            "--session",
            "aedev",
            "--session-id",
            "$7",
            "--pane",
            "%12",
            "--server-pid",
            "911",
            "--server-start",
            "1789109660",
        ];
        words.extend_from_slice(extra);
        words.into_iter().map(ToOwned::to_owned).collect()
    }

    #[test]
    fn a_confirm_argv_is_every_captured_fact_and_nothing_minted_yet() {
        let (step, captured) = parse(&argv("confirm", &[])).expect("the confirm grammar");
        assert_eq!(step, Step::Confirm);
        assert_eq!(captured.action, STOP);
        assert_eq!(captured.client, "/dev/ttys004");
        assert_eq!(captured.session_id, "$7");
        assert_eq!(captured.pane, "%12");
        assert_eq!(captured.server_start, "1789109660");
        assert!(
            captured.uuid.is_empty() && captured.deadline == 0,
            "confirm mints the identity snapshot and the deadline; it is not given them"
        );
    }

    #[test]
    fn confirm_refuses_to_be_handed_the_facts_it_is_supposed_to_mint() {
        let refusal = parse(&argv(
            "confirm",
            &["--uuid", "1b4e28ba-2fa1-11d2-883f-0016d3cc4321"],
        ))
        .expect_err("a minted field on confirm");
        assert!(matches!(refusal, Refusal::Usage(_)), "{refusal:?}");
    }

    #[test]
    fn an_apply_argv_carries_the_snapshot_and_the_deadline() {
        let (step, captured) = parse(&argv(
            "apply",
            &[
                "--uuid",
                "1B4E28BA-2FA1-11D2-883F-0016D3CC4321",
                "--deadline",
                "1789109780",
            ],
        ))
        .expect("the apply grammar");
        assert_eq!(step, Step::Apply);
        assert_eq!(captured.uuid, "1b4e28ba-2fa1-11d2-883f-0016d3cc4321");
        assert_eq!(captured.deadline, 1_789_109_780);
    }

    #[test]
    fn every_identity_field_is_an_allowlist() {
        for (flag, bad) in [
            ("--client", "/dev/tty;rm -rf /"),
            ("--client", "$(id)"),
            ("--session", "not a name"),
            ("--session", "ae#{session_name}"),
            ("--session-id", "7"),
            ("--session-id", "$"),
            ("--pane", "$12"),
            ("--server-pid", "-1"),
            ("--server-pid", "0"),
            ("--server-start", "x"),
            ("--client-pid", ""),
        ] {
            let mut words = argv("confirm", &[]);
            let at = words
                .iter()
                .position(|word| word == flag)
                .expect("the flag is in the fixture");
            words[at + 1] = bad.to_owned();
            let refusal = parse(&words).expect_err("{flag} {bad} must be refused");
            assert!(
                matches!(refusal, Refusal::Field(_)),
                "{flag} {bad:?}: {refusal:?}"
            );
        }
    }

    #[test]
    fn an_action_this_phase_does_not_offer_is_refused_rather_than_guessed() {
        let mut words = argv("confirm", &[]);
        let at = words
            .iter()
            .position(|word| word == "--action")
            .expect("the flag is in the fixture");
        words[at + 1] = "end".to_owned();
        assert!(matches!(
            parse(&words).expect_err("end is not offered yet"),
            Refusal::Field(_)
        ));
    }

    #[test]
    fn a_repeated_flag_is_an_ambiguity_not_a_last_one_wins() {
        let refusal = parse(&argv("confirm", &["--session", "other"]))
            .expect_err("two sessions is not a choice");
        assert!(matches!(refusal, Refusal::Usage(_)), "{refusal:?}");
    }

    /// tmux trims a row it cannot fit and draws the menu anyway, so a
    /// confirmation that fits by luck is a consequence the human never read.
    /// The budget is measured from the ROWS, never assumed from a constant.
    #[test]
    fn the_confirmation_budget_is_measured_from_its_own_rows() {
        let menu = stop_confirmation("aedev", "run-shell -b 'apply'");
        let (columns, rows) = menu_budget(&menu);
        let widest = menu
            .items
            .iter()
            .map(|item| item.label.chars().count())
            .max()
            .expect("the confirmation has rows");
        assert!(
            columns > widest,
            "{columns} columns cannot hold a {widest}-character row"
        );
        assert!(
            columns > 46,
            "46 was the guessed constant and it is too narrow for these rows: {columns}"
        );
        assert!(rows > menu.items.len(), "borders and the status line");

        // The exact target is part of the consequence, so a longer name needs
        // a wider client — a fixed budget could not know that.
        let long = "a".repeat(120);
        let (wide, _) = menu_budget(&stop_confirmation(&long, "run-shell -b 'apply'"));
        assert!(
            wide > columns + 100,
            "a {}-character session name must widen the budget: {wide} vs {columns}",
            long.len()
        );

        // The destructive row carries a key, and tmux spends columns on it.
        let keyed = menu
            .items
            .iter()
            .find(|item| !item.key.is_empty() && item.label.starts_with("Stop '"))
            .expect("the destructive row");
        assert!(
            columns >= keyed.label.chars().count() + keyed.key.chars().count() + 7,
            "the key column is part of the budget: {columns}"
        );
    }

    #[test]
    fn the_confirmation_puts_cancel_first_and_the_destructive_row_last() {
        let menu = stop_confirmation("aedev", "run-shell -b 'apply'");
        assert!(menu.title.contains("aedev"));
        let first = menu.items.first().expect("a first row");
        assert_eq!(first.label, "Cancel");
        assert!(
            matches!(&first.action, crate::tmux::MenuAction::Run(command) if command.is_empty()),
            "cancel queues nothing"
        );
        let last = menu.items.last().expect("a last row");
        assert_eq!(last.label, "Stop 'aedev' now");
        assert_eq!(last.key, "S");
        assert!(
            matches!(&last.action, crate::tmux::MenuAction::Run(command) if command == "run-shell -b 'apply'")
        );
        assert!(
            menu.items
                .iter()
                .any(|item| item.label.contains("PRESERVED")),
            "the consequence is on the menu, not only in the docs"
        );
        assert!(
            menu.items
                .iter()
                .filter(|item| matches!(item.action, crate::tmux::MenuAction::Run(_)))
                .count()
                == 2,
            "exactly two rows are choosable: cancel and the destructive one"
        );
    }

    #[test]
    fn pause_is_state_preserving_and_names_only_the_exact_resume_route() {
        for (session, route) in [
            ("orchestrator", "ae orchestrator --no-attach"),
            ("renamed", "ae renamed --no-attach"),
        ] {
            let menu = pause_confirmation(session, "run-shell -b 'apply'");
            assert!(
                menu.title.starts_with("Pause orchestrator"),
                "{}",
                menu.title
            );
            let first = menu.items.first().expect("a first row");
            assert_eq!(first.label, "Cancel");
            assert!(
                matches!(&first.action, crate::tmux::MenuAction::Run(command) if command.is_empty())
            );
            assert!(
                menu.items.iter().any(|item| item
                    .label
                    .contains("state, worktree and conversations are PRESERVED")),
                "pause consequence missing for {session}"
            );
            assert!(
                menu.items.iter().any(|item| item.label.contains(route)),
                "resume route missing for {session}"
            );
            let last = menu.items.last().expect("a last row");
            assert_eq!(last.label, format!("Pause '{session}' now"));
            assert_eq!(last.key, "P");
            assert!(
                matches!(&last.action, crate::tmux::MenuAction::Run(command) if command == "run-shell -b 'apply'")
            );
            assert_eq!(
                menu.items
                    .iter()
                    .filter(|item| matches!(item.action, crate::tmux::MenuAction::Run(_)))
                    .count(),
                2,
                "Cancel and Pause are the only choices"
            );
        }
    }

    #[test]
    fn pause_is_an_explicit_action_not_an_ordinary_stop_alias() {
        let mut words = argv("confirm", &[]);
        let at = words
            .iter()
            .position(|word| word == "--action")
            .expect("action flag");
        words[at + 1] = PAUSE_ORCHESTRATOR.to_owned();
        let (_, captured) = parse(&words).expect("pause grammar");
        assert_eq!(captured.action, PAUSE_ORCHESTRATOR);
    }
}
