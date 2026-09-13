//! `_session-menu` — the forward chain behind a right-click on one session's
//! status range.
//!
//! The steps, each its own process, each carrying the SAME captured facts as
//! literal arguments:
//!
//! 1. `show` reads the clicked session's declared state read-only, then takes
//!    ONE final clicker proof, then draws the centred root menu itself against
//!    the dimensions that proof returned; with no launcher the status binding
//!    keeps tmux's own native draw instead.
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

/// The chain's three steps, as the row or binding that queues each one spells
/// them.
pub const SHOW: &str = "show";
pub const CONFIRM: &str = "confirm";
pub const APPLY: &str = "apply";

/// The root menu's Flip row. The label and key are exactly the binding's own,
/// so a delegated draw is indistinguishable from the native one.
pub const FLIP_ROW_LABEL: &str = "Flip lead/colead panes";
pub const FLIP_ROW_KEY: &str = "f";

/// How many declaration rows the root draws before it counts the rest.
pub const STATE_ROWS_MAX: usize = 3;

/// The most cells one declaration's state value keeps in a root row.
const STATE_VALUE_CELLS: usize = 24;

/// The most cells one declaration's billing actor keeps in a root row.
const STATE_ACTOR_CELLS: usize = 24;

/// The most cells one declaration's reason keeps in a root row.
const STATE_REASON_CELLS: usize = 60;

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
pub const USAGE: &str = "Usage: _session-menu <show|confirm|apply> --client <name> --client-pid <pid> --session <name> --session-id <$id> --pane <%id> --server-pid <pid> --server-start <epoch> [show takes no more; confirm takes --action <stop|pause-orchestrator>; apply takes --action, --uuid <uuid> and --deadline <epoch>]";

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
    /// Prove the click, then draw the root menu read-only.
    Show,
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
        SHOW => Step::Show,
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
    if action
        .as_deref()
        .is_some_and(|action| action != STOP && action != PAUSE_ORCHESTRATOR)
    {
        return Err(Refusal::Field(format!(
            "unsupported action {:?} — this menu offers {STOP:?} or {PAUSE_ORCHESTRATOR:?}",
            action.as_deref().unwrap_or_default()
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
    let (action, uuid, deadline) = match step {
        Step::Show => {
            if action.is_some() || uuid.is_some() || deadline.is_some() {
                return Err(Refusal::Usage(
                    "show reads the captured facts alone; it takes no --action, --uuid or --deadline"
                        .to_owned(),
                ));
            }
            (String::new(), String::new(), 0)
        }
        Step::Confirm => {
            if uuid.is_some() || deadline.is_some() {
                return Err(Refusal::Usage(
                    "confirm mints --uuid and --deadline; it does not take them".to_owned(),
                ));
            }
            (action.ok_or_else(|| missing("--action"))?, String::new(), 0)
        }
        Step::Apply => {
            let action = action.ok_or_else(|| missing("--action"))?;
            let uuid = uuid.ok_or_else(|| missing("--uuid"))?;
            let canonical = crate::archive::canonical_uuid(&uuid);
            if canonical.is_empty() {
                return Err(Refusal::Field(format!("{uuid:?} is not a session uuid")));
            }
            let deadline = deadline.ok_or_else(|| missing("--deadline"))?;
            (action, canonical, positive("--deadline", &deadline)?)
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

/// What ONE `meta` read said — the correlation source, never read twice.
///
/// A non-regular node is classified BEFORE any open: a symlink is never
/// followed and a FIFO is never opened, so a hostile `meta` cannot block the
/// draw. The bytes are parsed only when they came from a regular file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaSource {
    /// No `meta` in the state directory.
    Absent,
    /// A `meta` that exists and could not be read, in the observed shape or as
    /// bytes. The reason is rendered into the gap row.
    Unreadable(String),
    /// The parsed roster names, and the canonical `session_id` the same bytes
    /// carry. An empty uuid means the document records no usable identity.
    Parsed {
        /// The canonical `session_id`, or empty.
        uuid: String,
        /// The roster actors, in meta order.
        actors: Vec<String>,
    },
}

impl MetaSource {
    /// Classify one session's meta: the node first, then the bytes, then the
    /// roster and the identity from those SAME bytes.
    fn read(dir: &Path) -> Self {
        match crate::store::read_source(&crate::store::open(dir).meta_path()) {
            crate::store::SourceRead::Absent => Self::Absent,
            crate::store::SourceRead::Invalid(reason) => Self::Unreadable(reason),
            crate::store::SourceRead::Unreadable(_) => Self::Unreadable("unreadable".to_owned()),
            crate::store::SourceRead::Ready(bytes) => Self::from_bytes(&bytes),
        }
    }

    /// The parsed view of a regular meta's bytes.
    fn from_bytes(bytes: &[u8]) -> Self {
        let parsed = crate::meta::Meta::parse(&String::from_utf8_lossy(bytes));
        // EXACTLY ONE session_id row: first-wins on a duplicated identity would
        // let the option match one row while the roster and the events render
        // from a document that says two things.
        let uuid = match crate::meta::sole_value(bytes, "session_id") {
            Some(value) => crate::archive::canonical_uuid(&String::from_utf8_lossy(value)),
            None if names_key(bytes, "session_id") => {
                return Self::Unreadable("duplicate identity".to_owned());
            }
            None => String::new(),
        };
        Self::Parsed {
            uuid,
            actors: parsed
                .roster()
                .iter()
                .map(crate::meta::RosterEntry::reference)
                .collect(),
        }
    }
}

/// Whether any well-formed `key=` row names `key`.
fn names_key(text: &[u8], key: &str) -> bool {
    let needle = format!("{key}=");
    text.split(|byte| *byte == b'\n').any(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        line.starts_with(needle.as_bytes())
    })
}

/// One row of the root menu's state section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootRow {
    /// A declared state: selectable, and choosing it does nothing.
    Declaration(String),
    /// A source gap: drawn dim, unselectable.
    Gap(String),
}

/// The state section of the root menu, from the sources as they were read.
///
/// Record-derived rows exist ONLY under a proven correlation: a nonempty
/// canonical UUID option and the same canonical UUID in the meta, read from the
/// state directory the click's session name addresses. Anything else renders a
/// truthful gap and never another incarnation's declarations.
#[must_use]
pub fn root_rows(
    option: &crate::tmux::OptionReading,
    meta: &MetaSource,
    events: &crate::store::SourceRead,
    now: crate::time::Timestamp,
) -> Vec<RootRow> {
    use crate::tmux::OptionReading;
    let uuid = match option {
        OptionReading::Set(value) => {
            let canonical = crate::archive::canonical_uuid(value);
            if canonical.is_empty() {
                return vec![RootRow::Gap(
                    "state: unavailable (session identity invalid)".to_owned(),
                )];
            }
            canonical
        }
        OptionReading::Vacant => {
            return vec![RootRow::Gap(
                "state: unavailable (session identity not recorded)".to_owned(),
            )];
        }
        OptionReading::Unknown => {
            return vec![RootRow::Gap(
                "state: unavailable (session identity unreadable)".to_owned(),
            )];
        }
    };
    let actors = match meta {
        MetaSource::Absent => {
            return vec![RootRow::Gap(
                "state: unavailable (meta: missing)".to_owned(),
            )];
        }
        MetaSource::Unreadable(reason) => {
            return vec![RootRow::Gap(format!(
                "state: unavailable (meta: {})",
                crate::event_text::display_cell(reason, STATE_REASON_CELLS)
            ))];
        }
        MetaSource::Parsed {
            uuid: meta_uuid,
            actors,
        } => {
            if meta_uuid.is_empty() {
                return vec![RootRow::Gap(
                    "state: unavailable (meta: no identity)".to_owned(),
                )];
            }
            if *meta_uuid != uuid {
                return vec![RootRow::Gap(
                    "state: unavailable (meta: identity mismatch)".to_owned(),
                )];
            }
            actors
        }
    };
    match events {
        crate::store::SourceRead::Invalid(reason)
        | crate::store::SourceRead::Unreadable(reason) => vec![RootRow::Gap(format!(
            "state: unreadable (events: {})",
            crate::event_text::display_cell(reason, STATE_REASON_CELLS)
        ))],
        crate::store::SourceRead::Absent => vec![RootRow::Gap("state: none declared".to_owned())],
        crate::store::SourceRead::Ready(bytes) => {
            let found = crate::state::latest_for_all(bytes, actors);
            if found.is_empty() {
                return vec![RootRow::Gap("state: none declared".to_owned())];
            }
            let mut rows: Vec<RootRow> = found
                .iter()
                .map(|(actor, latest)| RootRow::Declaration(declaration_label(actor, latest, now)))
                .collect();
            if rows.len() > STATE_ROWS_MAX {
                let dropped = rows.len() - STATE_ROWS_MAX;
                rows.truncate(STATE_ROWS_MAX);
                rows.push(RootRow::Gap(format!("+{dropped} more declarations")));
            }
            rows
        }
    }
}

/// One declaration as a root row: `<actor> state: <value> — <reason> (<age>)`,
/// with the reason omitted when the declaration carries none and every piece
/// projected through [`crate::event_text::display_cell`]. The actor is the
/// roster identity that declared it, so two actors' states cannot collapse
/// into indistinguishable rows.
fn declaration_label(
    actor: &str,
    latest: &crate::state::Latest,
    now: crate::time::Timestamp,
) -> String {
    let actor = crate::event_text::display_cell(actor, STATE_ACTOR_CELLS);
    let value =
        crate::event_text::display_cell(&String::from_utf8_lossy(&latest.value), STATE_VALUE_CELLS);
    let reason = crate::event_text::display_cell(
        &String::from_utf8_lossy(&latest.reason),
        STATE_REASON_CELLS,
    );
    let age = crate::brief::age(
        crate::time::Timestamp::parse(&String::from_utf8_lossy(&latest.ts))
            .map(|ts| ts.seconds_until(now)),
    );
    let mut label = format!("{actor} state: {value}");
    if !reason.is_empty() {
        label.push_str(" — ");
        label.push_str(&reason);
    }
    let _ = std::fmt::Write::write_fmt(&mut label, format_args!(" ({age})"));
    label
}

/// One row as a tmux item: declarations are selectable no-ops, gaps are dim.
fn root_row_item(row: &RootRow) -> crate::tmux::MenuItem {
    match row {
        RootRow::Declaration(label) => crate::tmux::MenuItem {
            label: label.clone(),
            key: String::new(),
            action: crate::tmux::MenuAction::Run(String::new()),
        },
        RootRow::Gap(label) => crate::tmux::MenuItem {
            label: label.clone(),
            key: String::new(),
            action: crate::tmux::MenuAction::Disabled,
        },
    }
}

fn root_separator() -> crate::tmux::MenuItem {
    crate::tmux::MenuItem {
        label: String::new(),
        key: String::new(),
        action: crate::tmux::MenuAction::Disabled,
    }
}

/// The Flip row. The action word is the SAME constant the binding's native
/// draw uses, passed verbatim: `display-menu` expands an item command when the
/// menu opens, so the doubled hashes stay doubled exactly once.
fn root_flip_item() -> crate::tmux::MenuItem {
    crate::tmux::MenuItem {
        label: FLIP_ROW_LABEL.to_owned(),
        key: FLIP_ROW_KEY.to_owned(),
        action: crate::tmux::MenuAction::Run(crate::tmux::MOUSE_DOWN_STATUS_MENU_ACTION.to_owned()),
    }
}

fn root_stop_item(stop: &str) -> crate::tmux::MenuItem {
    crate::tmux::MenuItem {
        label: STOP_ROW_LABEL.to_owned(),
        key: "s".to_owned(),
        action: crate::tmux::MenuAction::Run(stop.to_owned()),
    }
}

/// The full root: every status row, then the action floor.
#[must_use]
pub fn root_menu(session: &str, rows: &[RootRow], stop: Option<&str>) -> crate::tmux::Menu {
    let mut items: Vec<crate::tmux::MenuItem> = rows.iter().map(root_row_item).collect();
    items.push(root_separator());
    items.push(root_flip_item());
    if let Some(stop) = stop {
        items.push(root_stop_item(stop));
    }
    crate::tmux::Menu {
        title: session.to_owned(),
        title_style: String::new(),
        items,
    }
}

/// The degraded root: ONE status row and the action floor.
#[must_use]
pub fn status_only_menu(session: &str, row: &RootRow, stop: Option<&str>) -> crate::tmux::Menu {
    let mut items = vec![root_row_item(row), root_separator(), root_flip_item()];
    if let Some(stop) = stop {
        items.push(root_stop_item(stop));
    }
    crate::tmux::Menu {
        title: session.to_owned(),
        title_style: String::new(),
        items,
    }
}

/// Today's exact base menu: the title and the two actions, nothing else.
/// Below this floor tmux trims the way it always has; the root never refuses
/// once the clicker is proven.
#[must_use]
pub fn floor_menu(session: &str, stop: Option<&str>) -> crate::tmux::Menu {
    let mut items = vec![root_flip_item()];
    if let Some(stop) = stop {
        items.push(root_stop_item(stop));
    }
    crate::tmux::Menu {
        title: session.to_owned(),
        title_style: String::new(),
        items,
    }
}

/// Reselect the root against the FINAL live client dimensions: full, then
/// status-only, then today's floor. A build-dimension fit proves nothing about
/// the client the draw reaches.
#[must_use]
pub fn select_root(
    session: &str,
    rows: &[RootRow],
    stop: Option<&str>,
    client_width: usize,
    client_height: usize,
) -> crate::tmux::Menu {
    let full = root_menu(session, rows, stop);
    let (columns, lines) = menu_budget(&full);
    if client_width >= columns && client_height >= lines {
        return full;
    }
    if let Some(first) = rows.first() {
        let degraded = status_only_menu(session, first, stop);
        let (columns, lines) = menu_budget(&degraded);
        if client_width >= columns && client_height >= lines {
            return degraded;
        }
    }
    floor_menu(session, stop)
}

/// The Stop row's confirm argv: today's seven captured facts, one quoted word
/// each, and never a `--uuid`.
fn stop_row_command(captured: &Captured, launcher: &[String]) -> String {
    let mut argv = launcher.to_vec();
    argv.extend([crate::cli::SESSION_MENU, CONFIRM, "--action", STOP].map(ToOwned::to_owned));
    for (flag, value) in [
        ("--client", &captured.client),
        ("--client-pid", &captured.client_pid),
        ("--session", &captured.session),
        ("--session-id", &captured.session_id),
        ("--pane", &captured.pane),
        ("--server-pid", &captured.server_pid),
        ("--server-start", &captured.server_start),
    ] {
        argv.push(flag.to_owned());
        argv.push(value.clone());
    }
    crate::tmux::menu_run_shell_command(&argv)
}

/// Everything the root draw reads BEFORE the one clicker proof.
///
/// The type is the ordering: once this value exists, no source is read again.
/// The single [`prove_clicker`] follows, and the fit and the draw use the
/// dimensions THAT proof returned — never a snapshot taken before the reads,
/// so a resize during a 1.6 MB scan can only shrink the menu, not trim it.
struct ShowSources {
    server: ServerId,
    rows: Vec<RootRow>,
    stop: Option<String>,
    menu_mouse: bool,
}

/// Read every source, in the order the draw needs them, with NO proof yet.
///
/// A failure here is pre-proof: stderr alone, no draw and no client message.
fn read_sources(root: &Path, captured: &Captured, err: &mut impl Write) -> Option<ShowSources> {
    let Some(server) = crate::doors::caller_server() else {
        let _ = writeln!(
            err,
            "ae session menu: no calling tmux server, so ae cannot prove what was clicked."
        );
        return None;
    };
    let Some(core) = crate::shape::resolved_exe() else {
        let _ = writeln!(
            err,
            "ae session menu: ae cannot name its own executable, so it cannot offer the actions."
        );
        return None;
    };
    let option = crate::transport::observe_option_reading(
        &server,
        &captured.session_id,
        crate::theme::SESSION_ID_OPTION,
    );
    let dir = crate::lifecycle::sessions_dir(root).join(&captured.session);
    let meta = MetaSource::read(&dir);
    // The event container is read ONLY under a proven correlation: without it,
    // no event is attributable to this session incarnation.
    let correlated = matches!(
        (&option, &meta),
        (
            crate::tmux::OptionReading::Set(value),
            MetaSource::Parsed { uuid, .. }
        ) if crate::archive::canonical_uuid(value) == *uuid && !uuid.is_empty()
    );
    let events = if correlated {
        crate::store::open(&dir).events_source()
    } else {
        crate::store::SourceRead::Absent
    };
    let rows = root_rows(&option, &meta, &events, crate::time::Timestamp::now());
    let config = crate::doors::config_file(crate::shape::current(), root);
    let launcher = crate::session_tmux::picker_launcher(
        crate::shape::current(),
        &core,
        root,
        &config,
        &server,
    );
    let stop = (!launcher.is_empty()).then(|| stop_row_command(captured, &launcher));
    let menu_mouse = match crate::transport::probe_tmux_version(&server) {
        crate::tmux::VersionProbe::Answered(found) => {
            crate::tmux_floor::Probe::Server(found).menu_mouse()
        }
        crate::tmux::VersionProbe::NoServer | crate::tmux::VersionProbe::Unreachable => false,
    };
    Some(ShowSources {
        server,
        rows,
        stop,
        menu_mouse,
    })
}

/// `_session-menu show …` — read every source, prove the click ONCE, then draw
/// the root read-only.
///
/// Post-proof the root refuses neither ENRICHMENT nor FIT: every enrichment
/// failure became a row, and a menu the client cannot hold degrades to today's
/// floor. A clicker proof that fails is one failure, reported on stderr alone;
/// a draw tmux itself refuses is the other, reported to the client and exiting
/// nonzero.
fn run_show(root: &Path, captured: &Captured, err: &mut impl Write) -> u8 {
    let Some(sources) = read_sources(root, captured, err) else {
        return crate::entry::EXIT_FAILED;
    };
    // THE ONE PROOF, immediately before the fit and the draw; nothing reads
    // the world after it.
    let Some(clicker) = prove_clicker(captured, err) else {
        return crate::entry::EXIT_FAILED;
    };
    let menu = select_root(
        &captured.session,
        &sources.rows,
        sources.stop.as_deref(),
        clicker.client.width,
        clicker.client.height,
    );
    if !crate::transport::display_menu_centred(
        &sources.server,
        &captured.client,
        &captured.pane,
        &menu,
        sources.menu_mouse,
    ) {
        report(
            Some(&sources.server),
            captured,
            "tmux refused to draw the session menu; nothing was done.",
            err,
        );
        return crate::entry::EXIT_FAILED;
    }
    0
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
/// In `confirm` and `apply` this comes first: until ae knows which human is
/// owed the answer it cannot report a refusal, and until it knows the server no
/// other captured fact means anything. In `show` it is deliberately the ONE
/// FINAL proof, after every source read, so the fit and the draw use the live
/// dimensions it just returned and no world read follows it.
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
        Step::Show => Ok(run_show(root, &captured, err)),
        Step::Confirm => Ok(run_confirm(root, &captured, err)),
        Step::Apply => run_apply(root, &captured, out, err),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PAUSE_ORCHESTRATOR, Refusal, RootRow, STOP, STOP_ROW_LABEL, Step, menu_budget, parse,
        pause_confirmation, stop_confirmation,
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

    /// The show argv: the same seven captured facts, and no action.
    fn show_argv(extra: &[&str]) -> Vec<String> {
        let mut words: Vec<String> = [
            "show",
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
        ]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
        words.extend(extra.iter().map(|word| (*word).to_owned()));
        words
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

    /// The Flip row's action is the binding's exact constant, moved intact into
    /// the delegated draw.
    #[test]
    fn the_flip_row_is_the_exact_action_word() {
        let menu = super::floor_menu("aedev", Some("run-shell -b 'stop'"));
        let flip = menu.items.first().expect("the Flip row");
        assert_eq!(flip.label, super::FLIP_ROW_LABEL);
        assert_eq!(flip.key, super::FLIP_ROW_KEY);
        assert!(
            matches!(
                &flip.action,
                crate::tmux::MenuAction::Run(command)
                    if command == crate::tmux::MOUSE_DOWN_STATUS_MENU_ACTION
            ),
            "the Flip action word must be byte-identical to the binding's"
        );
    }

    /// The Stop row re-execs the SAME confirm step with the same seven facts
    /// today's binding splices, and never carries an identity of its own.
    #[test]
    fn the_stop_row_carries_every_action_fact_and_matches_todays_argv() {
        let captured = super::parse(&show_argv(&[])).expect("the show grammar").1;
        let launcher = vec!["/opt/ae".to_owned()];
        let stop = super::stop_row_command(&captured, &launcher);
        let menu = super::floor_menu("aedev", Some(&stop));
        let row = menu.items.last().expect("the Stop row");
        assert_eq!(row.label, STOP_ROW_LABEL);
        assert_eq!(row.key, "s");
        let crate::tmux::MenuAction::Run(command) = &row.action else {
            panic!("the Stop row runs something");
        };
        // BYTE-EXACT against an independently built argv: a flag/value check
        // alone stays green when two values are swapped, and a swapped pair
        // makes every Stop refuse at the proof.
        let expected = crate::tmux::menu_run_shell_command(
            &[
                "/opt/ae",
                crate::cli::SESSION_MENU,
                "confirm",
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
            ]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>(),
        );
        assert_eq!(
            command, &expected,
            "the Stop action must be byte-exact to today's seven-fact confirm argv"
        );
        assert!(
            !command.contains("--uuid") && !command.contains("--deadline"),
            "the root's Stop row mints neither identity nor deadline: {command}"
        );
    }

    /// The ladder is chosen AFTER the final proof: a fit computed at build
    /// dimensions must not survive a smaller live client.
    #[test]
    fn root_reselects_its_ladder_against_live_dimensions_and_never_refuses() {
        use crate::tmux::OptionReading;
        let now = crate::time::Timestamp::now();
        let container = concat!(
            r#"{"ts":"2026-09-13T08:00:00Z","actor":"builder","action":"state","ref":"working","summary":"one"}"#,
            "\n",
            r#"{"ts":"2026-09-13T08:00:01Z","actor":"lead","action":"state","ref":"blocked","summary":"two"}"#,
            "\n",
            r#"{"ts":"2026-09-13T08:00:02Z","actor":"colead","action":"state","ref":"waiting-user","summary":"three"}"#,
            "\n",
        );
        let rows = super::root_rows(
            &OptionReading::Set(UUID_A.to_owned()),
            &parsed_meta(UUID_A, &["builder", "lead", "colead"]),
            &events(container),
            now,
        );
        let stop = "run-shell -b 'stop'";
        let full = super::root_menu("aedev", &rows, Some(stop));
        let (full_columns, full_rows) = menu_budget(&full);
        assert!(
            full_rows >= 5,
            "the full menu is taller than the floor: {full_rows}"
        );
        let floor = super::floor_menu("aedev", Some(stop));
        let (_, floor_rows) = menu_budget(&floor);
        assert!(floor_rows < full_rows);

        // Built for a 200x50 client, drawn for a live 80x8 one: the LIVE
        // dimensions decide, and the full three-declaration menu no longer
        // fits while the action floor must survive.
        let live = super::select_root("aedev", &rows, Some(stop), 80, 8);
        let (live_columns, live_rows) = menu_budget(&live);
        assert!(
            live_columns <= 80 && live_rows <= 8,
            "the live-degraded variant must fit 80x8: {live_columns}x{live_rows}"
        );
        assert!(
            live_rows < full_rows,
            "a build-dimension fit would have picked the full menu: {live_rows}"
        );
        let labels: Vec<&str> = live.items.iter().map(|item| item.label.as_str()).collect();
        assert!(labels.contains(&super::FLIP_ROW_LABEL), "{labels:?}");
        assert!(labels.contains(&STOP_ROW_LABEL), "{labels:?}");
        assert_eq!(
            labels
                .iter()
                .filter(|label| label.contains(" state: "))
                .count(),
            1,
            "status-only keeps exactly one state row: {labels:?}"
        );

        // Below every variant, today's trim behaviour: a menu is still drawn.
        let tiny = super::select_root("aedev", &rows, Some(stop), 4, 2);
        let labels: Vec<&str> = tiny.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, vec![super::FLIP_ROW_LABEL, STOP_ROW_LABEL]);
        assert!(
            full_columns > 4,
            "the fixture really is wider than the tiny client"
        );
    }

    #[test]
    fn the_base_menu_budget_equals_todays_and_never_gains_a_refusal() {
        let stop = "run-shell -b 'apply'";
        let todays = crate::tmux::Menu {
            title: "aedev".to_owned(),
            title_style: String::new(),
            items: vec![
                crate::tmux::MenuItem {
                    label: super::FLIP_ROW_LABEL.to_owned(),
                    key: super::FLIP_ROW_KEY.to_owned(),
                    action: crate::tmux::MenuAction::Run(
                        crate::tmux::MOUSE_DOWN_STATUS_MENU_ACTION.to_owned(),
                    ),
                },
                crate::tmux::MenuItem {
                    label: STOP_ROW_LABEL.to_owned(),
                    key: "s".to_owned(),
                    action: crate::tmux::MenuAction::Run(stop.to_owned()),
                },
            ],
        };
        let floor = super::floor_menu("aedev", Some(stop));
        assert_eq!(
            menu_budget(&floor),
            menu_budget(&todays),
            "the floor IS today's base menu, measured by the one budget"
        );
        let (columns, rows) = menu_budget(&floor);
        // At exact budget and one row below it, the draw is still offered: the
        // root never refuses post-proof, and tmux trims as it always has.
        for (width, height) in [
            (columns, rows),
            (columns, rows - 1),
            (columns - 1, rows - 1),
        ] {
            let menu = super::select_root("aedev", &[], Some(stop), width, height);
            assert_eq!(menu.items.len(), floor.items.len(), "{width}x{height}");
        }
    }

    /// No dead entries, ever: Slice 1 ships the state section and the two
    /// actions ONLY.
    #[test]
    fn the_slice_1_root_never_offers_activity_or_memos() {
        let rows = vec![
            RootRow::Declaration("state: working — on it (3m)".to_owned()),
            RootRow::Declaration("state: blocked — waiting (41s)".to_owned()),
            RootRow::Gap("state: none declared".to_owned()),
        ];
        let variants = [
            super::root_menu("aedev", &rows, Some("run-shell -b 'stop'")),
            super::status_only_menu("aedev", &rows[0], Some("run-shell -b 'stop'")),
            super::floor_menu("aedev", Some("run-shell -b 'stop'")),
        ];
        for menu in variants {
            for item in &menu.items {
                assert!(
                    !item.label.contains("Activity") && !item.label.contains("Memos"),
                    "an entry without a working submenu must not exist: {:?}",
                    item.label
                );
            }
        }
    }

    #[test]
    fn show_reads_the_seven_facts_only_and_mints_nothing() {
        let (step, captured) = parse(&show_argv(&[])).expect("the show grammar");
        assert_eq!(step, Step::Show);
        assert_eq!(captured.session, "aedev");
        assert!(captured.action.is_empty() && captured.uuid.is_empty() && captured.deadline == 0);
        for extra in [
            vec!["--action", "stop"],
            vec!["--uuid", UUID_A],
            vec!["--deadline", "1789109780"],
        ] {
            assert!(
                matches!(parse(&show_argv(&extra)), Err(Refusal::Usage(_))),
                "show must refuse {extra:?}"
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

    // ── the root menu's state section ───────────────────────────────────────

    const UUID_A: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cc4321";

    fn parsed_meta(uuid: &str, actors: &[&str]) -> super::MetaSource {
        super::MetaSource::Parsed {
            uuid: uuid.to_owned(),
            actors: actors.iter().map(|actor| (*actor).to_owned()).collect(),
        }
    }

    fn declaration_rows(rows: &[RootRow]) -> Vec<&str> {
        rows.iter()
            .filter_map(|row| match row {
                RootRow::Declaration(label) => Some(label.as_str()),
                RootRow::Gap(_) => None,
            })
            .collect()
    }

    fn gap_rows(rows: &[RootRow]) -> Vec<&str> {
        rows.iter()
            .filter_map(|row| match row {
                RootRow::Gap(label) => Some(label.as_str()),
                RootRow::Declaration(_) => None,
            })
            .collect()
    }

    fn events(body: &str) -> crate::store::SourceRead {
        crate::store::SourceRead::Ready(body.as_bytes().to_vec())
    }

    /// The identity comparison is the whole guard against a same-name
    /// replacement rendering the other incarnation's declarations.
    #[test]
    fn root_state_rows_require_the_option_uuid_to_match_meta() {
        use crate::tmux::OptionReading;
        let now = crate::time::Timestamp::now();
        let container = concat!(
            r#"{"ts":"2026-09-13T08:00:00Z","actor":"builder","action":"state","ref":"waiting-user","summary":"decide the layout"}"#,
            "\n",
        );
        let rows = super::root_rows(
            &OptionReading::Set(UUID_A.to_owned()),
            &parsed_meta("fa4a9b3e-0000-4000-8000-000000000000", &["builder"]),
            &events(container),
            now,
        );
        assert!(
            declaration_rows(&rows).is_empty(),
            "a mismatched incarnation renders no declaration: {rows:?}"
        );
        assert_eq!(
            gap_rows(&rows),
            vec!["state: unavailable (meta: identity mismatch)"]
        );

        // The matching incarnation is the one that renders.
        let rows = super::root_rows(
            &OptionReading::Set(UUID_A.to_owned()),
            &parsed_meta(UUID_A, &["builder"]),
            &events(container),
            now,
        );
        let declared = declaration_rows(&rows);
        assert_eq!(declared.len(), 1, "{rows:?}");
        assert!(
            declared[0].starts_with("builder state: waiting-user — decide the layout ("),
            "{declared:?}"
        );
    }

    /// No option at all: no record-derived state, and the action floor is
    /// untouched.
    #[test]
    fn root_without_a_session_uuid_option_draws_the_floor() {
        use crate::tmux::OptionReading;
        let now = crate::time::Timestamp::now();
        for option in [OptionReading::Vacant, OptionReading::Unknown] {
            let rows = super::root_rows(
                &option,
                &parsed_meta(UUID_A, &["builder"]),
                &events(""),
                now,
            );
            assert!(
                declaration_rows(&rows).is_empty(),
                "{option:?} must not render declarations: {rows:?}"
            );
            assert_eq!(gap_rows(&rows).len(), 1, "{rows:?}");
            let menu = super::select_root("aedev", &rows, Some("run-shell -b 'stop'"), 200, 60);
            assert!(
                menu.items
                    .iter()
                    .any(|item| item.label == super::FLIP_ROW_LABEL),
                "the floor's Flip row survives: {rows:?}"
            );
            assert!(
                menu.items
                    .iter()
                    .any(|item| matches!(&item.action, crate::tmux::MenuAction::Run(command) if command == "run-shell -b 'stop'")),
                "the floor's Stop row survives"
            );
        }
    }

    /// The quiet event read answers an unreadable container with empty bytes;
    /// a human read must say the gap instead of claiming nobody declared.
    #[test]
    fn root_with_unreadable_events_says_unreadable_never_none_declared() {
        use crate::tmux::OptionReading;
        let now = crate::time::Timestamp::now();
        for corrupt in [
            crate::store::SourceRead::Invalid("a directory".to_owned()),
            crate::store::SourceRead::Unreadable("permission denied".to_owned()),
        ] {
            let rows = super::root_rows(
                &OptionReading::Set(UUID_A.to_owned()),
                &parsed_meta(UUID_A, &["builder"]),
                &corrupt,
                now,
            );
            let gaps = gap_rows(&rows);
            assert_eq!(gaps.len(), 1, "{rows:?}");
            assert!(
                gaps[0].starts_with("state: unreadable (events: "),
                "the explicit gap names the source and the reason: {gaps:?}"
            );
            assert!(
                !gaps[0].contains("none declared"),
                "an unreadable container is never rendered as an empty one"
            );
        }
    }

    #[test]
    fn root_with_absent_events_says_none_declared() {
        use crate::tmux::OptionReading;
        let now = crate::time::Timestamp::now();
        let rows = super::root_rows(
            &OptionReading::Set(UUID_A.to_owned()),
            &parsed_meta(UUID_A, &["builder"]),
            &crate::store::SourceRead::Absent,
            now,
        );
        assert!(declaration_rows(&rows).is_empty(), "{rows:?}");
        assert_eq!(gap_rows(&rows), vec!["state: none declared"]);
    }

    /// A state directory whose meta cannot be read is a correlation gap: no
    /// record-derived state, but the action floor still has to be offered.
    #[test]
    fn root_with_unreadable_meta_keeps_flip_and_stop_and_shows_no_record_state() {
        use crate::tmux::OptionReading;
        let now = crate::time::Timestamp::now();
        let rows = super::root_rows(
            &OptionReading::Set(UUID_A.to_owned()),
            &super::MetaSource::Unreadable("unreadable".to_owned()),
            &crate::store::SourceRead::Absent,
            now,
        );
        assert!(declaration_rows(&rows).is_empty(), "{rows:?}");
        assert_eq!(
            gap_rows(&rows),
            vec!["state: unavailable (meta: unreadable)"]
        );
        let menu = super::select_root("aedev", &rows, Some("run-shell -b 'stop'"), 200, 60);
        let actions: Vec<&str> = menu
            .items
            .iter()
            .filter_map(|item| match &item.action {
                crate::tmux::MenuAction::Run(command) => Some(command.as_str()),
                crate::tmux::MenuAction::Disabled => None,
            })
            .collect();
        assert!(
            actions
                .iter()
                .any(|command| command.contains(crate::tmux::MOUSE_DOWN_STATUS_MENU_ACTION)),
            "the Flip row keeps its exact action word: {actions:?}"
        );
        assert!(actions.contains(&"run-shell -b 'stop'"), "{actions:?}");
    }

    /// A duplicated `session_id` is an ambiguous identity, not a first-wins
    /// one: the option could match the first row while the roster and the
    /// events render from a document that says two things.
    #[test]
    fn a_duplicate_session_id_is_ambiguous_and_renders_no_record_state() {
        use crate::tmux::OptionReading;
        let now = crate::time::Timestamp::now();
        let container = concat!(
            r#"{"ts":"2026-09-13T08:00:00Z","actor":"lead","action":"state","ref":"working","summary":"x"}"#,
            "\n",
        );
        for (label, meta) in [
            (
                "equal",
                format!("session_id={UUID_A}\nsession_id={UUID_A}\n"),
            ),
            (
                "conflicting",
                format!("session_id={UUID_A}\nsession_id=fa4a9b3e-0000-4000-8000-000000000000\n"),
            ),
        ] {
            let source = super::MetaSource::from_bytes(meta.as_bytes());
            assert_eq!(
                source,
                super::MetaSource::Unreadable("duplicate identity".to_owned()),
                "{label}: a duplicated identity is ambiguous"
            );
            let rows = super::root_rows(
                &OptionReading::Set(UUID_A.to_owned()),
                &source,
                &events(container),
                now,
            );
            assert!(declaration_rows(&rows).is_empty(), "{label}: {rows:?}");
            assert_eq!(
                gap_rows(&rows),
                vec!["state: unavailable (meta: duplicate identity)"],
                "{label}"
            );
        }
        // The control: the SAME identity in ONE row is parsed and correlated.
        let sole = super::MetaSource::from_bytes(
            format!("session_id={UUID_A}\nseat.main=lead\n").as_bytes(),
        );
        assert!(
            matches!(&sole, super::MetaSource::Parsed { uuid, .. } if uuid == UUID_A),
            "{sole:?}"
        );
        // A bare `session_id` with no `=` is not a row, so it is not a rival.
        let bare =
            super::MetaSource::from_bytes(format!("session_id={UUID_A}\nsession_id\n").as_bytes());
        assert!(matches!(bare, super::MetaSource::Parsed { .. }), "{bare:?}");
    }

    /// The meta node is classified BEFORE any open: a non-regular one is a
    /// named gap, never followed and never blocked on.
    #[test]
    fn a_nonregular_meta_node_is_classified_before_any_open() {
        let dir = std::path::PathBuf::from(format!("/tmp/ae-menu-meta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(super::MetaSource::read(&dir), super::MetaSource::Absent);
        let meta = dir.join("meta");
        std::fs::create_dir_all(&meta).unwrap();
        assert_eq!(
            super::MetaSource::read(&dir),
            super::MetaSource::Unreadable("a directory".to_owned())
        );
        std::fs::remove_dir_all(&meta).unwrap();
        let target = dir.join("elsewhere");
        std::fs::write(&target, "session_id=x\n").unwrap();
        std::os::unix::fs::symlink(&target, &meta).unwrap();
        assert_eq!(
            super::MetaSource::read(&dir),
            super::MetaSource::Unreadable("a symlink".to_owned()),
            "a symlink is never followed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Per-roster provenance: two actors' declarations must stay
    /// distinguishable, so each row names its declaring actor.
    #[test]
    fn every_declaration_row_names_its_own_actor() {
        use crate::tmux::OptionReading;
        let now = crate::time::Timestamp::now();
        let container = concat!(
            r#"{"ts":"2026-09-13T08:00:00Z","actor":"builder","action":"state","ref":"blocked","summary":"one"}"#,
            "\n",
            r#"{"ts":"2026-09-13T08:00:01Z","actor":"lead","action":"state","ref":"working","summary":"two"}"#,
            "\n",
        );
        let rows = super::root_rows(
            &OptionReading::Set(UUID_A.to_owned()),
            &parsed_meta(UUID_A, &["builder", "lead"]),
            &events(container),
            now,
        );
        let labels = declaration_rows(&rows);
        assert_eq!(labels.len(), 2, "{rows:?}");
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("builder state: blocked — one (")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("lead state: working — two (")),
            "{labels:?}"
        );
    }
}
