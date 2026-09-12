//! The explicit-client settings menu and its orchestrator-role control.
//!
//! The menu is observational. Its launch rows carry a captured clicker and an
//! expected role state to the existing launch owner; Pause uses the existing
//! session-menu confirmation chain.

use std::io::Write;
use std::path::Path;

use crate::inventory::ServerId;
use crate::meta::{MetaAgentRole, ServerSelector};
use crate::tmux::{Menu, MenuAction, MenuItem, StopProbe};

/// The settings-only launch continuation marker.
pub(crate) const APPLY_FLAG: &str = "--settings-apply";

/// A raw metadata record included in the complete role census.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RoleRecord {
    name: String,
    meta: Vec<u8>,
}

/// What a complete raw role census found.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RoleSelection {
    Zero,
    One(String),
    Unavailable(String),
}

/// The durable identity of the single role target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoleTarget {
    pub(crate) name: String,
    pub(crate) uuid: String,
    pub(crate) server: ServerId,
    pub(crate) pane: String,
}

/// What the orchestrator control can honestly offer at draw time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Control {
    Start,
    Resume(RoleTarget),
    Pause {
        target: RoleTarget,
        session_id: String,
    },
    Unavailable(String),
}

/// Identity captured for the one client that opened the menu.
pub(crate) struct Snapshot<'a> {
    pub(crate) client: &'a str,
    pub(crate) client_pid: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) server_pid: &'a str,
    pub(crate) server_start: &'a str,
    pub(crate) deadline: i64,
}

fn select_role(records: &[RoleRecord]) -> RoleSelection {
    let mut role = None;
    for record in records {
        match crate::meta::meta_agent_role(&record.meta) {
            MetaAgentRole::Absent => {}
            MetaAgentRole::Role if role.is_none() => role = Some(record.name.clone()),
            MetaAgentRole::Role => {
                return RoleSelection::Unavailable(
                    "multiple sessions claim the orchestrator role".to_owned(),
                );
            }
            MetaAgentRole::Damaged => {
                return RoleSelection::Unavailable(format!(
                    "session '{}' has a damaged meta_agent role claim",
                    record.name
                ));
            }
        }
    }
    role.map_or(RoleSelection::Zero, RoleSelection::One)
}

fn role_records(root: &Path) -> Result<Vec<RoleRecord>, String> {
    let names = crate::lifecycle::census(root)
        .map_err(|why| format!("the session census failed ({why})"))?
        .unwrap_or_default();
    names
        .into_iter()
        .map(|name| {
            let dir = crate::lifecycle::sessions_dir(root).join(&name);
            crate::meta::read_bytes(&dir)
                .map(|meta| RoleRecord {
                    name: name.clone(),
                    meta,
                })
                .map_err(|why| format!("metadata for session '{name}' is unreadable ({why})"))
        })
        .collect()
}

/// Authoritative Start proof, called only while the canonical lifecycle lock
/// is held. The canonical lock serializes canonical starts and renames through
/// that name; it deliberately makes no global claim about separately
/// authorized creation under a different name.
pub(crate) fn prove_absent_start(root: &Path, launch_server: &ServerId) -> Result<(), String> {
    let records = role_records(root)?;
    match select_role(&records) {
        RoleSelection::Zero => {}
        RoleSelection::One(name) => {
            return Err(format!(
                "orchestrator role is now recorded by '{name}'; stale Start refused"
            ));
        }
        RoleSelection::Unavailable(why) => return Err(why),
    }
    let canonical =
        crate::lifecycle::sessions_dir(root).join(crate::orchestrator::ORCHESTRATOR_SESSION);
    if crate::lifecycle::path_exists(&canonical) {
        return Err("canonical orchestrator state is no longer absent".to_owned());
    }
    match crate::transport::verify_session_absent(
        launch_server,
        crate::orchestrator::ORCHESTRATOR_SESSION,
    ) {
        StopProbe::Absent => Ok(()),
        StopProbe::Present => Err("canonical tmux namesake is no longer absent".to_owned()),
        StopProbe::Unknown => {
            Err("canonical tmux absence cannot be proven on the launch server".to_owned())
        }
    }
}

/// Authoritative Resume proof, called only while the captured target's
/// lifecycle lock is held. It re-censuses the role and rejects a live,
/// replaced, renamed or duplicated target instead of changing the action.
pub(crate) fn prove_stopped_role(
    root: &Path,
    name: &str,
    expected_uuid: &str,
) -> Result<ServerId, String> {
    let records = role_records(root)?;
    match select_role(&records) {
        RoleSelection::One(found) if found == name => {}
        RoleSelection::One(found) => {
            return Err(format!(
                "orchestrator role moved from '{name}' to '{found}'; stale Resume refused"
            ));
        }
        RoleSelection::Zero => {
            return Err(format!(
                "'{name}' no longer records the orchestrator role; stale Resume refused"
            ));
        }
        RoleSelection::Unavailable(why) => return Err(why),
    }
    let Some(record) = records.iter().find(|record| record.name == name) else {
        return Err(format!(
            "role target '{name}' vanished during the locked census"
        ));
    };
    let target = target_from(name.to_owned(), &record.meta)?;
    if target.uuid != expected_uuid {
        return Err(format!(
            "role target '{name}' was replaced since the settings menu opened"
        ));
    }
    match crate::transport::verify_session_absent(&target.server, name) {
        StopProbe::Absent => Ok(target.server),
        StopProbe::Present => Err(format!(
            "role target '{name}' is live now; stale Resume refused"
        )),
        StopProbe::Unknown => Err(format!(
            "the recorded server cannot prove role target '{name}' stopped"
        )),
    }
}

fn target_from(name: String, meta: &[u8]) -> Result<RoleTarget, String> {
    let uuid = crate::archive::canonical_uuid(&crate::lifecycle::meta_value(meta, "session_id"));
    if uuid.is_empty() {
        return Err(format!(
            "role target '{name}' has no canonical session identity"
        ));
    }
    let ServerSelector::Positive(selector) = crate::lifecycle::server_of(meta) else {
        return Err(format!(
            "role target '{name}' has no unambiguous recorded tmux server"
        ));
    };
    let pane = crate::lifecycle::meta_value(meta, "main_pane");
    if !crate::tmux::pane_id_is_valid(&pane) {
        return Err(format!(
            "role target '{name}' has no canonical recorded lead pane"
        ));
    }
    Ok(RoleTarget {
        name,
        uuid,
        server: ServerId::Selected(selector),
        pane,
    })
}

/// Classify the current orchestrator role without writing or migrating state.
pub(crate) fn discover(
    root: &Path,
    invoking_server: &ServerId,
    launch_server: &ServerId,
) -> Control {
    let records = match role_records(root) {
        Ok(records) => records,
        Err(why) => return Control::Unavailable(why),
    };
    let name = match select_role(&records) {
        RoleSelection::Zero => {
            let canonical = crate::lifecycle::sessions_dir(root)
                .join(crate::orchestrator::ORCHESTRATOR_SESSION);
            if crate::lifecycle::path_exists(&canonical) {
                return Control::Unavailable(
                    "the canonical 'orchestrator' state exists without the orchestrator role"
                        .to_owned(),
                );
            }
            return match crate::transport::verify_session_absent(
                launch_server,
                crate::orchestrator::ORCHESTRATOR_SESSION,
            ) {
                StopProbe::Absent => Control::Start,
                StopProbe::Present => Control::Unavailable(
                    "a live canonical namesake exists without proven orchestrator state".to_owned(),
                ),
                StopProbe::Unknown => Control::Unavailable(
                    "the canonical launch server could not prove the orchestrator name absent"
                        .to_owned(),
                ),
            };
        }
        RoleSelection::One(name) => name,
        RoleSelection::Unavailable(why) => return Control::Unavailable(why),
    };
    let Some(record) = records.iter().find(|record| record.name == name) else {
        return Control::Unavailable("the role census changed while it was read".to_owned());
    };
    let target = match target_from(name, &record.meta) {
        Ok(target) => target,
        Err(why) => return Control::Unavailable(why),
    };
    match crate::transport::verify_session_absent(&target.server, &target.name) {
        StopProbe::Absent => Control::Resume(target),
        StopProbe::Unknown => Control::Unavailable(format!(
            "the recorded server cannot prove role target '{}' stopped",
            target.name
        )),
        StopProbe::Present => {
            let mut sockets = crate::SocketPaths::asking(crate::transport::observe_socket_path);
            if !sockets.proven_same(invoking_server, &target.server) {
                return Control::Unavailable(format!(
                    "role target '{}' is live on another tmux server",
                    target.name
                ));
            }
            let Some(session_id) = crate::lifecycle::live_id(&target.server, &target.name) else {
                return Control::Unavailable(format!(
                    "role target '{}' changed while its liveness was read",
                    target.name
                ));
            };
            Control::Pause { target, session_id }
        }
    }
}

fn launch_argv(
    launcher: &[String],
    action: &str,
    target: &str,
    uuid: &str,
    snapshot: &Snapshot<'_>,
) -> Vec<String> {
    let mut argv = launcher.to_vec();
    argv.extend([
        crate::orchestrator::ORCHESTRATOR_SESSION.to_owned(),
        APPLY_FLAG.to_owned(),
        action.to_owned(),
        "--target".to_owned(),
        target.to_owned(),
        "--uuid".to_owned(),
        uuid.to_owned(),
        "--client".to_owned(),
        snapshot.client.to_owned(),
        "--client-pid".to_owned(),
        snapshot.client_pid.to_owned(),
        "--server-pid".to_owned(),
        snapshot.server_pid.to_owned(),
        "--server-start".to_owned(),
        snapshot.server_start.to_owned(),
        "--deadline".to_owned(),
        snapshot.deadline.to_string(),
    ]);
    argv
}

fn pause_argv(
    launcher: &[String],
    target: &RoleTarget,
    session_id: &str,
    snapshot: &Snapshot<'_>,
) -> Vec<String> {
    let mut argv = launcher.to_vec();
    argv.extend([
        crate::cli::SESSION_MENU.to_owned(),
        crate::session_menu::CONFIRM.to_owned(),
        "--action".to_owned(),
        crate::session_menu::PAUSE_ORCHESTRATOR.to_owned(),
        "--client".to_owned(),
        snapshot.client.to_owned(),
        "--client-pid".to_owned(),
        snapshot.client_pid.to_owned(),
        "--session".to_owned(),
        target.name.clone(),
        "--session-id".to_owned(),
        session_id.to_owned(),
        "--pane".to_owned(),
        target.pane.clone(),
        "--server-pid".to_owned(),
        snapshot.server_pid.to_owned(),
        "--server-start".to_owned(),
        snapshot.server_start.to_owned(),
    ]);
    argv
}

/// Build the one-action settings menu. Drawing or dismissing it writes nothing.
pub(crate) fn menu(
    control: &Control,
    launcher: &[String],
    snapshot: &Snapshot<'_>,
    version: Option<&str>,
    palette: &crate::theme::Palette,
) -> Menu {
    let (status, label, key, action) = match control {
        Control::Start => (
            "orchestrator: absent".to_owned(),
            "Start orchestrator".to_owned(),
            "s".to_owned(),
            MenuAction::Run(crate::tmux::menu_run_shell_command(&launch_argv(
                launcher,
                "start",
                crate::orchestrator::ORCHESTRATOR_SESSION,
                "",
                snapshot,
            ))),
        ),
        Control::Resume(target) => (
            format!("orchestrator: stopped ({})", target.name),
            format!("Resume orchestrator '{}'", target.name),
            "r".to_owned(),
            MenuAction::Run(crate::tmux::menu_run_shell_command(&launch_argv(
                launcher,
                "resume",
                &target.name,
                &target.uuid,
                snapshot,
            ))),
        ),
        Control::Pause { target, session_id } => (
            format!("orchestrator: running ({})", target.name),
            format!("Pause orchestrator '{}'...", target.name),
            "p".to_owned(),
            MenuAction::Run(crate::tmux::menu_run_shell_command(&pause_argv(
                launcher, target, session_id, snapshot,
            ))),
        ),
        Control::Unavailable(why) => (
            "orchestrator: unavailable".to_owned(),
            why.clone(),
            String::new(),
            MenuAction::Disabled,
        ),
    };
    let (label, key, action) = match action {
        MenuAction::Run(command) if crate::tmux::session_id_is_valid(snapshot.session_id) => {
            (label, key, MenuAction::Run(command))
        }
        MenuAction::Run(_) => (
            "settings action unavailable: invoking session identity is invalid".to_owned(),
            String::new(),
            MenuAction::Disabled,
        ),
        MenuAction::Disabled => (label, key, MenuAction::Disabled),
    };
    let mut items = vec![
        MenuItem {
            label: status,
            key: String::new(),
            action: MenuAction::Disabled,
        },
        MenuItem {
            label: String::new(),
            key: String::new(),
            action: MenuAction::Disabled,
        },
        MenuItem { label, key, action },
    ];
    // THE property, not a site: every live row of the finished menu carries
    // the marker unset, applied here after the label rewrite above — so the
    // rewrite is already priced into any budget taken of this menu.
    ensure_settings_unset(&mut items, snapshot.session_id);
    Menu {
        title: title(version),
        title_style: crate::theme::menu_title_style(palette),
        items,
    }
}

/// Prefix every live row with the settings-marker unset, skipping rows that
/// already carry it. The ONE place the invariant lives: each settings menu
/// variant ends here, so whichever menu is finally drawn — base, full or
/// degraded — has no live row that can bypass it. With an invalid invoking
/// identity there are no live rows to arm, and none is invented.
fn ensure_settings_unset(items: &mut [MenuItem], session_id: &str) {
    if !crate::tmux::session_id_is_valid(session_id) {
        return;
    }
    let prefix = format!(
        "set-option -u -t {session_id} {} ; ",
        crate::theme::SETTINGS_OPEN_OPTION
    );
    for item in items {
        if let MenuAction::Run(command) = &mut item.action
            && !command.starts_with(&prefix)
        {
            *command = format!("{prefix}{command}");
        }
    }
}

/// The quota dialog's Close row: dismissing the menu and choosing it are the
/// same act, so closing the dialog writes nothing anywhere.
fn close_item() -> MenuItem {
    MenuItem {
        label: "Close".to_owned(),
        key: "c".to_owned(),
        // Cancel QUEUES NOTHING. Dismissing the menu and choosing this
        // row must be the same act.
        action: MenuAction::Run(String::new()),
    }
}

/// The quota dialog's title: what the menu is, not which core drew it.
pub(crate) const QUOTA_DIALOG_TITLE: &str = "Client quotas";

/// The one live quota row in the settings menu. It opens the centred
/// per-window dialog; it never acts on quota itself. With an invalid invoking
/// identity it is a keyless disabled reason, never a row that looks live.
pub(crate) fn quota_entry(launcher: &[String], snapshot: &Snapshot<'_>) -> MenuItem {
    if !crate::tmux::session_id_is_valid(snapshot.session_id) {
        return MenuItem {
            label: "quota unavailable: invoking session identity is invalid".to_owned(),
            key: String::new(),
            action: MenuAction::Disabled,
        };
    }
    let mut argv = launcher.to_vec();
    argv.extend(
        [
            crate::orchestrator::ORCHESTRATOR_SESSION,
            "--quota-dialog",
            "--client",
            snapshot.client,
            "--client-pid",
            snapshot.client_pid,
            "--server-pid",
            snapshot.server_pid,
            "--server-start",
            snapshot.server_start,
            "--session-id",
            snapshot.session_id,
        ]
        .map(ToOwned::to_owned),
    );
    MenuItem {
        label: "Client quotas...".to_owned(),
        key: "q".to_owned(),
        action: MenuAction::Run(crate::tmux::menu_run_shell_command(&argv)),
    }
}

/// Place the one live quota entry above the existing control section.
pub(crate) fn menu_with_quota(
    control: &Control,
    launcher: &[String],
    snapshot: &Snapshot<'_>,
    version: Option<&str>,
    palette: &crate::theme::Palette,
    entry: MenuItem,
) -> Menu {
    let mut built = menu(control, launcher, snapshot, version, palette);
    let mut items = Vec::with_capacity(built.items.len() + 2);
    items.push(entry);
    items.push(MenuItem {
        label: String::new(),
        key: String::new(),
        action: MenuAction::Disabled,
    });
    items.append(&mut built.items);
    built.items = items;
    ensure_settings_unset(&mut built.items, snapshot.session_id);
    built
}

/// The settings menu for one awareness: the base control menu with no quota
/// entry anywhere when unaware, otherwise the quota entry above it. The ONE
/// place the settings surface reads awareness — the caller resolves the bool
/// through `config::quota_aware` and this branch enforces it.
pub(crate) fn menu_for_awareness(
    quota_aware: bool,
    control: &Control,
    launcher: &[String],
    snapshot: &Snapshot<'_>,
    version: Option<&str>,
    palette: &crate::theme::Palette,
    entry: MenuItem,
) -> Menu {
    if !quota_aware {
        return menu(control, launcher, snapshot, version, palette);
    }
    menu_with_quota(control, launcher, snapshot, version, palette, entry)
}

/// Build the observational quota dialog: informational rows are selectable
/// no-ops, so tmux opens on the first data row and Enter dismisses it. The
/// blank separator stays disabled.
pub(crate) fn quota_dialog_menu(
    rows: &[crate::quota::DialogRow],
    palette: &crate::theme::Palette,
) -> Menu {
    let mut items = Vec::with_capacity(rows.len() + 2);
    items.extend(rows.iter().map(|row| {
        debug_assert!(
            !row.label.starts_with('-'),
            "quota dialog rows must not start with tmux's disabled marker"
        );
        MenuItem {
            label: row.label.clone(),
            key: String::new(),
            action: MenuAction::Run(String::new()),
        }
    }));
    items.push(MenuItem {
        label: String::new(),
        key: String::new(),
        action: MenuAction::Disabled,
    });
    items.push(close_item());
    Menu {
        title: QUOTA_DIALOG_TITLE.to_owned(),
        title_style: crate::theme::menu_title_style(palette),
        items,
    }
}

/// Replace the existing blank separator with a bounded quota overflow notice.
/// The item count stays identical to the base menu, so an exact-height client
/// never loses the orchestrator action to the informational section.
pub(crate) fn menu_with_quota_notice(
    control: &Control,
    launcher: &[String],
    snapshot: &Snapshot<'_>,
    version: Option<&str>,
    palette: &crate::theme::Palette,
    missing: (usize, usize),
    base_columns: usize,
) -> Menu {
    let mut built = menu(control, launcher, snapshot, version, palette);
    let number = |value: usize| {
        if value > 9 {
            "9+".to_owned()
        } else {
            value.to_string()
        }
    };
    let notice = format!("+{}r +{}c", number(missing.0), number(missing.1));
    let max_label = base_columns.saturating_sub(4);
    let Some(empty_separator) = built.items.get_mut(1) else {
        return built;
    };
    debug_assert!(empty_separator.label.is_empty());
    empty_separator.label = notice.chars().take(max_label).collect();
    ensure_settings_unset(&mut built.items, snapshot.session_id);
    built
}

/// The title names only a complete session-published ae `CalVer`.
///
/// The session fact may belong to an older core during an upgrade, so it is
/// intentionally not replaced with the binary building this menu. A valid but
/// long fact remains a truthful title; the existing menu budget then visibly
/// refuses clients too narrow to draw it.
fn title(version: Option<&str>) -> String {
    let Some(version) = version else {
        return "ae settings".to_owned();
    };
    let Some(calver) = version.strip_prefix("ae ") else {
        return "ae settings".to_owned();
    };
    if !crate::install::is_version(calver) {
        return "ae settings".to_owned();
    }
    format!("{version} settings")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchAction {
    Start,
    Resume,
}

struct CapturedLaunch {
    action: LaunchAction,
    target: String,
    uuid: String,
    client: String,
    client_pid: String,
    server_pid: String,
    server_start: String,
    deadline: i64,
    #[cfg(debug_assertions)]
    test_pre_lock_marker: bool,
}

/// Whether the public orchestrator word carries the settings continuation.
pub(crate) fn is_apply(tail: &[String]) -> bool {
    tail.first().is_some_and(|word| word == APPLY_FLAG)
}

/// Whether `text` names an addressable tmux client.
///
/// The ONE grammar every continuation carrying a client name requires — the
/// settings apply and the quota dialog both read it, so a second spelling of
/// "addressable" cannot drift in beside it.
pub(crate) fn is_client_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._/:+-".contains(&byte))
}

/// The quota-dialog continuation marker: a second hop off the settings menu.
pub(crate) const QUOTA_DIALOG_FLAG: &str = "--quota-dialog";

/// A captured quota-dialog invocation: the client name plus the pid pair and
/// the viewed session that prove it is still the same clicker.
///
/// The dialog writes nothing, so there is no decision window to bound and no
/// deadline travels with it. Identity is what travels: a reused client name or
/// socket is a replacement, a switched session reads the wrong overlay, and
/// either is refused, never drawn for.
pub(crate) struct CapturedQuotaDialog {
    pub(crate) client: String,
    pub(crate) client_pid: String,
    pub(crate) server_pid: String,
    pub(crate) server_start: String,
    pub(crate) session_id: String,
}

/// Read the fixed hostile quota-dialog grammar: each flag exactly once, the
/// client through [`is_client_name`], the pids decimal, the session id through
/// the session grammar. A read-only draw has no `--deadline` because there is
/// no stale effect to bound.
pub(crate) fn parse_quota_dialog(tail: &[String]) -> Result<CapturedQuotaDialog, String> {
    let [flag, rest @ ..] = tail else {
        return Err("incomplete quota dialog invocation".to_owned());
    };
    if flag != QUOTA_DIALOG_FLAG {
        return Err("not a quota dialog invocation".to_owned());
    }
    let mut client = None;
    let mut client_pid = None;
    let mut server_pid = None;
    let mut server_start = None;
    let mut session_id = None;
    let mut remaining = rest;
    while let [flag, after @ ..] = remaining {
        let Some((value, after)) = after.split_first() else {
            return Err("a quota dialog flag is missing its value".to_owned());
        };
        let slot = match flag.as_str() {
            "--client" => &mut client,
            "--client-pid" => &mut client_pid,
            "--server-pid" => &mut server_pid,
            "--server-start" => &mut server_start,
            "--session-id" => &mut session_id,
            other => return Err(format!("unknown quota dialog flag {other:?}")),
        };
        if slot.replace(value.clone()).is_some() {
            return Err(format!("{flag} may be given only once"));
        }
        remaining = after;
    }
    let required =
        |value: Option<String>, flag: &str| value.ok_or_else(|| format!("{flag} is required"));
    let client = required(client, "--client")?;
    if !is_client_name(&client) {
        return Err("--client is not an addressable tmux client".to_owned());
    }
    let decimal = |value: Option<String>, flag: &str| -> Result<String, String> {
        let value = required(value, flag)?;
        if !crate::tmux::is_decimal(&value) {
            return Err(format!("{flag} is not a decimal"));
        }
        Ok(value)
    };
    let client_pid = decimal(client_pid, "--client-pid")?;
    let server_pid = decimal(server_pid, "--server-pid")?;
    let server_start = decimal(server_start, "--server-start")?;
    let session_id = required(session_id, "--session-id")?;
    if !crate::tmux::session_id_is_valid(&session_id) {
        return Err("--session-id is not a tmux session identity".to_owned());
    }
    Ok(CapturedQuotaDialog {
        client,
        client_pid,
        server_pid,
        server_start,
        session_id,
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "one fixed hostile settings-continuation grammar"
)]
fn parse_apply(tail: &[String]) -> Result<CapturedLaunch, String> {
    let [flag, action, rest @ ..] = tail else {
        return Err("incomplete settings action".to_owned());
    };
    if flag != APPLY_FLAG {
        return Err("not a settings action".to_owned());
    }
    let action = match action.as_str() {
        "start" => LaunchAction::Start,
        "resume" => LaunchAction::Resume,
        other => return Err(format!("unknown settings action {other:?}")),
    };
    let mut target = None;
    let mut uuid = None;
    let mut client = None;
    let mut client_pid = None;
    let mut server_pid = None;
    let mut server_start = None;
    let mut deadline = None;
    #[cfg(debug_assertions)]
    let mut test_pre_lock_marker = false;
    let mut remaining = rest;
    while let [flag, after @ ..] = remaining {
        if flag == "--test-pre-lock-marker" {
            #[cfg(debug_assertions)]
            {
                if test_pre_lock_marker {
                    return Err("--test-pre-lock-marker may be given only once".to_owned());
                }
                test_pre_lock_marker = true;
                remaining = after;
                continue;
            }
            #[cfg(not(debug_assertions))]
            return Err(format!("unknown settings flag {flag:?}"));
        }
        let Some((value, after)) = after.split_first() else {
            return Err("a settings flag is missing its value".to_owned());
        };
        let slot = match flag.as_str() {
            "--target" => &mut target,
            "--uuid" => &mut uuid,
            "--client" => &mut client,
            "--client-pid" => &mut client_pid,
            "--server-pid" => &mut server_pid,
            "--server-start" => &mut server_start,
            "--deadline" => &mut deadline,
            other => return Err(format!("unknown settings flag {other:?}")),
        };
        if slot.replace(value.clone()).is_some() {
            return Err(format!("{flag} may be given only once"));
        }
        remaining = after;
    }
    let required =
        |value: Option<String>, flag: &str| value.ok_or_else(|| format!("{flag} is required"));
    let target = required(target, "--target")?;
    if !crate::lifecycle::name_is_valid(&target) {
        return Err("--target is not an ae session name".to_owned());
    }
    let uuid = required(uuid, "--uuid")?;
    if action == LaunchAction::Start {
        if target != crate::orchestrator::ORCHESTRATOR_SESSION || !uuid.is_empty() {
            return Err("Start must name the absent canonical seat".to_owned());
        }
    } else if crate::archive::canonical_uuid(&uuid).is_empty() {
        return Err("--uuid is not a session identity".to_owned());
    }
    let client = required(client, "--client")?;
    if !is_client_name(&client) {
        return Err("--client is not an addressable tmux client".to_owned());
    }
    let decimal = |value: Option<String>, flag: &str| -> Result<String, String> {
        let value = required(value, flag)?;
        if !crate::tmux::is_decimal(&value) {
            return Err(format!("{flag} is not a decimal"));
        }
        Ok(value)
    };
    let client_pid = decimal(client_pid, "--client-pid")?;
    let server_pid = decimal(server_pid, "--server-pid")?;
    let server_start = decimal(server_start, "--server-start")?;
    let deadline = required(deadline, "--deadline")?
        .parse::<i64>()
        .map_err(|_| "--deadline is not an epoch second".to_owned())?;
    Ok(CapturedLaunch {
        action,
        target,
        uuid,
        client,
        client_pid,
        server_pid,
        server_start,
        deadline,
        #[cfg(debug_assertions)]
        test_pre_lock_marker,
    })
}

fn report(
    server: Option<&ServerId>,
    expectation: Option<&crate::session_launch::ExpectedLaunch>,
    captured: &CapturedLaunch,
    text: &str,
    err: &mut impl Write,
) {
    if let (Some(server), Some(expectation)) = (server, expectation)
        && expectation.check_attachment().is_ok()
    {
        let _ = crate::transport::display_client_message(server, &captured.client, text);
    }
    let _ = writeln!(err, "ae settings: {text}");
}

/// Apply Start/Resume through the ordinary launch owner with an expectation.
#[allow(
    clippy::too_many_lines,
    reason = "one action from captured identity proof through exact-client report"
)]
pub(crate) fn run_apply(
    preamble: &crate::entry::Preamble,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let captured = match parse_apply(tail) {
        Ok(captured) => captured,
        Err(why) => {
            writeln!(err, "ae settings: {why}.")?;
            return Ok(crate::entry::EXIT_USAGE);
        }
    };
    let Some(server) = preamble.caller_server.clone() else {
        report(
            None,
            None,
            &captured,
            "no calling tmux server; nothing was done",
            err,
        );
        return Ok(crate::entry::EXIT_FAILED);
    };
    let expectation = crate::session_launch::ExpectedLaunch::new(
        match captured.action {
            LaunchAction::Start => crate::session_launch::ExpectedState::AbsentCanonical,
            LaunchAction::Resume => crate::session_launch::ExpectedState::StoppedRole {
                uuid: crate::archive::canonical_uuid(&captured.uuid),
            },
        },
        server.clone(),
        captured.server_pid.clone(),
        captured.server_start.clone(),
        captured.client.clone(),
        captured.client_pid.clone(),
        captured.deadline,
    );
    #[cfg(debug_assertions)]
    let expectation = {
        let mut expectation = expectation;
        if captured.test_pre_lock_marker {
            expectation.enable_test_pre_lock_marker();
        }
        expectation
    };
    if let Err(why) = expectation.check_action(crate::time::Timestamp::now().epoch()) {
        report(
            Some(&server),
            Some(&expectation),
            &captured,
            &format!("{why}; nothing was done"),
            err,
        );
        return Ok(crate::entry::EXIT_FAILED);
    }
    let deps = crate::doctor::check_deps(&[], err)?;
    if deps != 0 {
        report(
            Some(&server),
            Some(&expectation),
            &captured,
            "launch dependencies are unavailable; nothing was done",
            err,
        );
        return Ok(deps);
    }
    let mut seat = preamble.clone();
    seat.attach = false;
    let user = match captured.action {
        LaunchAction::Start => {
            seat.local = Some(preamble.home.join(crate::orchestrator::CONFIG_FILE));
            crate::orchestrator::seat_launch_args()
        }
        LaunchAction::Resume => vec![captured.target.clone()],
    };
    let mut launch_out = Vec::new();
    let mut launch_err = Vec::new();
    let code = crate::session_launch::run_expected(
        &seat,
        &user,
        &expectation,
        &mut launch_out,
        &mut launch_err,
    )?;
    out.write_all(&launch_out)?;
    err.write_all(&launch_err)?;
    if code == 0 {
        let message = match captured.action {
            LaunchAction::Start => "Started orchestrator without switching this client.".to_owned(),
            LaunchAction::Resume => format!(
                "Resumed orchestrator '{}' without switching this client.",
                captured.target
            ),
        };
        if expectation.check_attachment().is_ok() {
            let _ = crate::transport::display_client_message(&server, &captured.client, &message);
        }
    } else {
        let detail = String::from_utf8_lossy(&launch_err)
            .lines()
            .next()
            .unwrap_or("launch refused")
            .to_owned();
        if expectation.check_attachment().is_ok() {
            let _ = crate::transport::display_client_message(&server, &captured.client, &detail);
        }
        if launch_err.is_empty() {
            writeln!(err, "ae settings: {detail}")?;
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        Control, LaunchAction, RoleRecord, RoleSelection, RoleTarget, Snapshot, menu, parse_apply,
        select_role,
    };
    use crate::inventory::ServerId;
    use crate::meta::Selector;

    const UUID: &str = "33333333-3333-4333-8333-333333333333";

    fn target(name: &str) -> RoleTarget {
        RoleTarget {
            name: name.to_owned(),
            uuid: UUID.to_owned(),
            server: ServerId::Selected(Selector::Socket(PathBuf::from("/tmp/ae role,sock"))),
            pane: "%12".to_owned(),
        }
    }

    fn snapshot() -> Snapshot<'static> {
        Snapshot {
            client: "/dev/ttys004",
            client_pid: "4242",
            session_id: "$7",
            server_pid: "911",
            server_start: "1789109660",
            deadline: 1_789_109_780,
        }
    }

    fn action(control: &Control) -> String {
        let built = menu(
            control,
            &[
                "env".to_owned(),
                "AE_HOME=/tmp/ae home#,}".to_owned(),
                Path::new("/tmp/ae core#,}").display().to_string(),
            ],
            &snapshot(),
            None,
            &crate::theme::Palette::DARCULA,
        );
        match &built.items[2].action {
            crate::tmux::MenuAction::Run(command) => command.clone(),
            crate::tmux::MenuAction::Disabled => String::new(),
        }
    }

    #[test]
    fn target_role_not_invoking_role_selects_the_orchestrator() {
        let records = [
            RoleRecord {
                name: "ordinary".to_owned(),
                meta: b"session=ordinary\n".to_vec(),
            },
            RoleRecord {
                name: "renamed".to_owned(),
                meta: b"session=renamed\nmeta_agent=true\n".to_vec(),
            },
        ];
        assert_eq!(
            select_role(&records),
            RoleSelection::One("renamed".to_owned())
        );
    }

    #[test]
    fn zero_canonical_and_renamed_censuses_have_distinct_exact_answers() {
        assert_eq!(select_role(&[]), RoleSelection::Zero);
        for name in ["orchestrator", "renamed"] {
            assert_eq!(
                select_role(&[RoleRecord {
                    name: name.to_owned(),
                    meta: b"meta_agent=true\n".to_vec(),
                }]),
                RoleSelection::One(name.to_owned())
            );
        }
    }

    #[test]
    fn a_role_target_needs_saved_identity_server_and_lead_pane() {
        let valid = format!(
            "meta_agent=true\nsession_id={UUID}\ntmux_server_kind=socket\ntmux_server=/tmp/s\nmain_pane=%2\n"
        );
        let found = super::target_from("renamed".to_owned(), valid.as_bytes())
            .expect("complete role target");
        assert_eq!(found.name, "renamed");
        assert_eq!(found.uuid, UUID);
        assert_eq!(found.pane, "%2");

        for broken in [
            valid.replace(UUID, "not-a-uuid"),
            valid.replace("tmux_server_kind=socket", "tmux_server_kind=ambiguous"),
            valid.replace("main_pane=%2", "main_pane=2"),
        ] {
            assert!(super::target_from("renamed".to_owned(), broken.as_bytes()).is_err());
        }
    }

    #[test]
    fn duplicate_or_multiple_role_claims_refuse() {
        let damaged = [RoleRecord {
            name: "one".to_owned(),
            meta: b"meta_agent=true\nmeta_agent=true\n".to_vec(),
        }];
        assert!(matches!(
            select_role(&damaged),
            RoleSelection::Unavailable(_)
        ));

        let multiple = [
            RoleRecord {
                name: "one".to_owned(),
                meta: b"meta_agent=true\n".to_vec(),
            },
            RoleRecord {
                name: "two".to_owned(),
                meta: b"meta_agent=true\n".to_vec(),
            },
        ];
        assert!(matches!(
            select_role(&multiple),
            RoleSelection::Unavailable(_)
        ));
    }

    #[test]
    fn absent_role_is_ordinary_but_false_or_malformed_is_damage() {
        assert_eq!(
            crate::meta::meta_agent_role(b"session=ordinary\n"),
            crate::meta::MetaAgentRole::Absent
        );
        assert_eq!(
            crate::meta::meta_agent_role(b"meta_agent=true\n"),
            crate::meta::MetaAgentRole::Role
        );
        assert_eq!(
            crate::meta::meta_agent_role(b"meta_agent=false\n"),
            crate::meta::MetaAgentRole::Damaged
        );
        assert_eq!(
            crate::meta::meta_agent_role(b"meta_agent\n"),
            crate::meta::MetaAgentRole::Damaged
        );
    }

    #[test]
    fn each_control_builds_one_exact_action_and_unavailable_builds_none() {
        let start = action(&Control::Start);
        assert!(
            start.starts_with("set-option -u -t $7 @ae_settings_open ; "),
            "{start}"
        );
        assert!(start.contains("'--settings-apply' 'start'"), "{start}");
        assert!(start.contains("'--target' 'orchestrator'"), "{start}");
        assert_eq!(start.matches("'--settings-apply'").count(), 1, "{start}");
        assert!(!start.contains("--test-pre-lock-marker"), "{start}");

        let renamed = target("renamed");
        let resume = action(&Control::Resume(renamed.clone()));
        assert!(resume.contains("'--settings-apply' 'resume'"), "{resume}");
        assert!(resume.contains("'--target' 'renamed'"), "{resume}");
        assert!(resume.contains(UUID), "{resume}");

        let pause = action(&Control::Pause {
            target: renamed,
            session_id: "$7".to_owned(),
        });
        assert!(pause.contains("'_session-menu' 'confirm'"), "{pause}");
        assert!(pause.contains("'--action' 'pause-orchestrator'"), "{pause}");
        assert!(pause.contains("'--session' 'renamed'"), "{pause}");
        assert_eq!(pause.matches("'--session'").count(), 1, "{pause}");
        assert!(pause.contains("'--session-id' '\\$7'"), "{pause}");
        assert_eq!(pause.matches("'--action'").count(), 1, "{pause}");

        assert!(action(&Control::Unavailable("damaged role".to_owned())).is_empty());
    }

    #[test]
    fn invalid_invoking_session_identity_renders_a_keyless_reason_not_an_action() {
        let mut invalid = snapshot();
        invalid.session_id = "not-a-session-id";
        let built = menu(
            &Control::Start,
            &[],
            &invalid,
            None,
            &crate::theme::Palette::DARCULA,
        );
        let action = &built.items[2];
        assert_eq!(
            action.label,
            "settings action unavailable: invoking session identity is invalid"
        );
        assert!(action.key.is_empty());
        assert!(matches!(action.action, crate::tmux::MenuAction::Disabled));
    }

    #[test]
    fn quota_entry_is_one_live_row_above_the_unchanged_control() {
        let entry = super::quota_entry(&["core".to_owned()], &snapshot());
        assert_eq!(entry.label, "Client quotas...");
        assert_eq!(entry.key, "q");
        let crate::tmux::MenuAction::Run(command) = &entry.action else {
            panic!("the quota entry is live");
        };
        // The entry itself carries no unset: `menu_with_quota` arms every live
        // row from the one site, so the property holds for rows `menu` never saw.
        assert!(!command.contains("set-option"), "{command}");
        assert!(command.contains("'--quota-dialog'"), "{command}");
        assert!(command.contains("'--client' '/dev/ttys004'"), "{command}");
        assert!(command.contains("'--client-pid' '4242'"), "{command}");
        assert!(command.contains("'--server-pid' '911'"), "{command}");
        assert!(
            command.contains("'--server-start' '1789109660'"),
            "{command}"
        );
        assert!(
            command.contains("'--session-id' '\\$7'"),
            "the viewed session travels escaped like every other menu command: {command}"
        );
        assert!(!command.contains("--pane"), "{command}");
        assert!(!command.contains("--deadline"), "{command}");
        let built = super::menu_with_quota(
            &Control::Start,
            &[],
            &snapshot(),
            None,
            &crate::theme::Palette::DARCULA,
            entry,
        );
        assert_eq!(built.items[0].label, "Client quotas...");
        assert_eq!(built.items[0].key, "q");
        let crate::tmux::MenuAction::Run(armed) = &built.items[0].action else {
            panic!("the assembled entry stays live");
        };
        assert!(
            armed.starts_with("set-option -u -t $7 @ae_settings_open ; "),
            "{armed}"
        );
        assert!(built.items[1].label.is_empty());
        assert_eq!(built.items[2].label, "orchestrator: absent");
        assert_eq!(built.items[4].label, "Start orchestrator");
        assert_eq!(built.items[4].key, "s");
    }

    /// The awareness property: unaware carries no quota entry row anywhere in
    /// the menu, aware keeps exactly the one live row above the control.
    #[test]
    fn awareness_decides_whether_the_quota_entry_row_exists() {
        let snapshot = snapshot();
        let entry_for = |aware: bool| {
            super::menu_for_awareness(
                aware,
                &Control::Start,
                &[],
                &snapshot,
                None,
                &crate::theme::Palette::DARCULA,
                super::quota_entry(&[], &snapshot),
            )
        };
        let unaware = entry_for(false);
        assert!(
            unaware
                .items
                .iter()
                .all(|item| !item.label.to_lowercase().contains("quota")),
            "unaware menu must carry no quota row: {:?}",
            unaware
                .items
                .iter()
                .map(|item| &item.label)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            unaware.items.len(),
            menu(
                &Control::Start,
                &[],
                &snapshot,
                None,
                &crate::theme::Palette::DARCULA,
            )
            .items
            .len(),
            "unaware menu is exactly the base control menu"
        );
        let aware = entry_for(true);
        assert_eq!(aware.items[0].label, "Client quotas...");
        assert_eq!(aware.items[0].key, "q");
        assert!(
            matches!(aware.items[0].action, crate::tmux::MenuAction::Run(_)),
            "the aware entry stays live"
        );
    }

    #[test]
    fn invalid_invoking_session_identity_disables_the_quota_entry_too() {
        let mut invalid = snapshot();
        invalid.session_id = "not-a-session-id";
        let entry = super::quota_entry(&[], &invalid);
        assert_eq!(
            entry.label,
            "quota unavailable: invoking session identity is invalid"
        );
        assert!(entry.key.is_empty());
        assert!(matches!(entry.action, crate::tmux::MenuAction::Disabled));
        let built = super::menu_with_quota(
            &Control::Start,
            &[],
            &invalid,
            None,
            &crate::theme::Palette::DARCULA,
            entry,
        );
        assert!(built.items[0].key.is_empty());
        assert!(matches!(
            built.items[0].action,
            crate::tmux::MenuAction::Disabled
        ));
    }

    /// The property, pinned for all three menus: no live row without the
    /// marker unset. `menu_with_quota` must arm the row it adds itself,
    /// because `menu` cannot see it.
    #[test]
    fn every_live_row_of_every_settings_menu_carries_the_marker_unset() {
        fn armed(menu: &crate::tmux::Menu) -> Vec<String> {
            menu.items
                .iter()
                .filter_map(|item| match &item.action {
                    crate::tmux::MenuAction::Run(command) => Some(command.clone()),
                    crate::tmux::MenuAction::Disabled => None,
                })
                .collect()
        }
        let snapshot = snapshot();
        let base = menu(
            &Control::Start,
            &[],
            &snapshot,
            None,
            &crate::theme::Palette::DARCULA,
        );
        let full = super::menu_with_quota(
            &Control::Start,
            &[],
            &snapshot,
            None,
            &crate::theme::Palette::DARCULA,
            super::quota_entry(&[], &snapshot),
        );
        let degraded = super::menu_with_quota_notice(
            &Control::Start,
            &[],
            &snapshot,
            None,
            &crate::theme::Palette::DARCULA,
            (3, 7),
            crate::session_menu::menu_budget(&base).0,
        );
        // The notice pins its shortfall SHAPE, not a bare plus: both halves.
        assert!(
            degraded.items[1].label.contains("+3r") && degraded.items[1].label.contains("+7c"),
            "notice: {:?}",
            degraded.items[1].label
        );
        for (name, menu) in [("base", base), ("full", full), ("degraded", degraded)] {
            let live = armed(&menu);
            assert!(!live.is_empty(), "{name} has a live row to arm");
            for command in live {
                assert!(
                    command.starts_with("set-option -u -t $7 @ae_settings_open ; "),
                    "{name}: {command}"
                );
                assert_eq!(
                    command
                        .matches("set-option -u -t $7 @ae_settings_open ; ")
                        .count(),
                    1,
                    "{name}: unset exactly once: {command}"
                );
            }
        }
    }

    #[test]
    fn quota_dialog_menu_rows_are_inert_and_undimmed_with_one_close_row() {
        let rows = [
            crate::quota::DialogRow {
                label: "codex/cx".to_owned(),
            },
            crate::quota::DialogRow {
                label: "  session 5h | 39% | window resets 3h | seen 0m | fresh".to_owned(),
            },
        ];
        let built = super::quota_dialog_menu(&rows, &crate::theme::Palette::DARCULA);
        assert_eq!(built.title, super::QUOTA_DIALOG_TITLE);
        assert_eq!(built.items.len(), rows.len() + 2);
        for item in &built.items[..rows.len()] {
            assert!(item.key.is_empty());
            assert!(matches!(
                item.action,
                crate::tmux::MenuAction::Run(ref command) if command.is_empty()
            ));
        }
        let separator = &built.items[rows.len()];
        assert!(separator.label.is_empty());
        assert!(matches!(
            separator.action,
            crate::tmux::MenuAction::Disabled
        ));
        let close = &built.items[rows.len() + 1];
        assert_eq!(close.label, "Close");
        assert_eq!(close.key, "c");
        assert!(matches!(
            close.action,
            crate::tmux::MenuAction::Run(ref command) if command.is_empty()
        ));
    }

    #[test]
    fn quota_dialog_information_rows_render_without_the_dim_marker() {
        let rows = [
            crate::quota::DialogRow {
                label: "codex/cx".to_owned(),
            },
            crate::quota::DialogRow {
                label: "  session 5h | 39%".to_owned(),
            },
        ];
        let menu = super::quota_dialog_menu(&rows, &crate::theme::Palette::DARCULA);
        let server = ServerId::Selected(Selector::Socket(PathBuf::from("/tmp/ae-quota.sock")));
        let argv =
            crate::tmux::display_menu_centred_args(&server, "/dev/ttys004", "%12", &menu, false);
        let first_item = argv
            .iter()
            .position(|word| word == "--")
            .expect("menu items follow the flag separator")
            + 1;
        for (index, row) in rows.iter().enumerate() {
            let label = &argv[first_item + index * 3];
            assert_eq!(label, &row.label, "quota row {index} is not dimmed");
            assert!(!label.starts_with('-'), "quota row {index}: {label}");
        }
    }

    #[test]
    #[should_panic(expected = "quota dialog rows must not start with tmux's disabled marker")]
    fn quota_dialog_rejects_a_row_that_would_reintroduce_tmux_dimming() {
        let rows = [crate::quota::DialogRow {
            label: "-would be dimmed".to_owned(),
        }];
        let _ = super::quota_dialog_menu(&rows, &crate::theme::Palette::DARCULA);
    }

    #[test]
    fn quota_labels_use_concise_client_wording() {
        let entry = super::quota_entry(&[], &snapshot());
        assert_eq!(entry.label, "Client quotas...");
        assert_eq!(super::QUOTA_DIALOG_TITLE, "Client quotas");
    }

    #[test]
    fn disabled_settings_rows_keep_tmuxs_dim_marker() {
        let menu = menu(
            &Control::Unavailable("orchestrator cannot start".to_owned()),
            &[],
            &snapshot(),
            None,
            &crate::theme::Palette::DARCULA,
        );
        let server = ServerId::Selected(Selector::Socket(PathBuf::from("/tmp/ae-quota.sock")));
        let argv =
            crate::tmux::display_menu_centred_args(&server, "/dev/ttys004", "%12", &menu, false);
        let first_item = argv
            .iter()
            .position(|word| word == "--")
            .expect("menu items follow the flag separator")
            + 1;
        for (index, label) in ["orchestrator: unavailable", "", "orchestrator cannot start"]
            .iter()
            .enumerate()
        {
            assert_eq!(
                argv[first_item + index * 3],
                format!("-{label}"),
                "disabled settings row {index} stays dimmed"
            );
            assert!(argv[first_item + index * 3 + 1].is_empty());
            assert!(argv[first_item + index * 3 + 2].is_empty());
        }
    }

    #[test]
    fn quota_overflow_notice_reuses_the_separator_and_never_grows_the_base_budget() {
        let base = menu(
            &Control::Start,
            &[],
            &snapshot(),
            None,
            &crate::theme::Palette::DARCULA,
        );
        let base_budget = crate::session_menu::menu_budget(&base);
        let degraded = super::menu_with_quota_notice(
            &Control::Start,
            &[],
            &snapshot(),
            None,
            &crate::theme::Palette::DARCULA,
            (123, 456),
            base_budget.0,
        );
        assert_eq!(degraded.items.len(), base.items.len());
        assert_eq!(degraded.items[1].label, "+9+r +9+c");
        assert!(matches!(
            degraded.items[1].action,
            crate::tmux::MenuAction::Disabled
        ));
        assert!(degraded.items[1].key.is_empty());
        assert_eq!(crate::session_menu::menu_budget(&degraded), base_budget);
        assert_eq!(degraded.items[2].label, "Start orchestrator");
        assert_eq!(degraded.items[2].key, "s");
    }

    #[test]
    fn concise_entry_menu_budget_never_widens_the_base_menu() {
        let full = super::menu_with_quota(
            &Control::Start,
            &[],
            &snapshot(),
            None,
            &crate::theme::Palette::DARCULA,
            super::quota_entry(&[], &snapshot()),
        );
        let base = menu(
            &Control::Start,
            &[],
            &snapshot(),
            None,
            &crate::theme::Palette::DARCULA,
        );
        let (full_columns, full_rows) = crate::session_menu::menu_budget(&full);
        let (base_columns, base_rows) = crate::session_menu::menu_budget(&base);
        assert_eq!(full_rows, base_rows + 2, "one entry row plus its separator");
        assert!(
            full_columns < 80,
            "the entry row fits ordinary clients: {full_columns}"
        );
        assert_eq!(
            full_columns, base_columns,
            "the concise entry cannot create a column shortfall after the base-fit check"
        );
    }

    #[test]
    fn settings_title_uses_only_the_exact_session_version_fact() {
        let other_core = "ae 2099.1.2";
        assert_ne!(other_core, crate::version_line());
        let long_calver = format!("ae {}.1.1", "9".repeat(500));
        let cases = [
            (Some(other_core), "ae 2099.1.2 settings"),
            (None, "ae settings"),
            (Some(""), "ae settings"),
            (Some("AE 2026.9.52"), "ae settings"),
            (Some(" ae 2026.9.52"), "ae settings"),
            (Some("ae  2026.9.52"), "ae settings"),
            (Some("ae 2026.9"), "ae settings"),
            (Some("ae 2026.9.x"), "ae settings"),
            (Some("ae 2026.9.52\n"), "ae settings"),
            (Some("ae 2026.9.52\0"), "ae settings"),
        ];
        for (version, expected) in cases {
            assert_eq!(super::title(version), expected, "{version:?}");
        }
        let long_title = super::title(Some(&long_calver));
        assert_eq!(long_title, format!("{long_calver} settings"));
        let menu = menu(
            &Control::Start,
            &[],
            &snapshot(),
            Some(&long_calver),
            &crate::theme::Palette::DARCULA,
        );
        assert!(crate::session_menu::menu_budget(&menu).0 > 140);
    }

    #[test]
    fn settings_apply_accepts_only_canonical_start_or_exact_identity_resume() {
        let args = |action: &str, target: &str, uuid: &str| {
            [
                "--settings-apply",
                action,
                "--target",
                target,
                "--uuid",
                uuid,
                "--client",
                "/dev/ttys004",
                "--client-pid",
                "4242",
                "--server-pid",
                "911",
                "--server-start",
                "1789109660",
                "--deadline",
                "1789109780",
            ]
            .map(ToOwned::to_owned)
        };
        let start = parse_apply(&args("start", "orchestrator", "")).expect("canonical Start");
        assert_eq!(start.action, LaunchAction::Start);
        assert_eq!(start.target, "orchestrator");
        let resume = parse_apply(&args("resume", "renamed", UUID)).expect("exact Resume");
        assert_eq!(resume.action, LaunchAction::Resume);
        assert_eq!(resume.target, "renamed");
        assert_eq!(resume.uuid, UUID);
        assert!(parse_apply(&args("start", "renamed", "")).is_err());
        assert!(parse_apply(&args("resume", "renamed", "not-a-uuid")).is_err());

        #[cfg(debug_assertions)]
        {
            let mut marked = args("start", "orchestrator", "").to_vec();
            marked.push("--test-pre-lock-marker".to_owned());
            assert!(parse_apply(&marked).is_ok());
            marked.push("--test-pre-lock-marker".to_owned());
            assert!(parse_apply(&marked).is_err());
        }
        #[cfg(not(debug_assertions))]
        {
            let mut marked = args("start", "orchestrator", "").to_vec();
            marked.push("--test-pre-lock-marker".to_owned());
            assert!(parse_apply(&marked).is_err());
        }

        let mut duplicate = args("resume", "renamed", UUID).to_vec();
        duplicate.extend(["--target".to_owned(), "other".to_owned()]);
        assert!(parse_apply(&duplicate).is_err());
    }

    #[test]
    fn quota_dialog_accepts_only_named_client_and_decimal_pid_pair_without_deadline() {
        let args = || {
            [
                "--quota-dialog",
                "--client",
                "/dev/ttys004",
                "--client-pid",
                "4242",
                "--server-pid",
                "911",
                "--server-start",
                "1789109660",
                "--session-id",
                "$7",
            ]
            .map(ToOwned::to_owned)
            .to_vec()
        };
        let captured = super::parse_quota_dialog(&args()).expect("canonical dialog");
        assert_eq!(captured.client, "/dev/ttys004");
        assert_eq!(captured.client_pid, "4242");
        assert_eq!(captured.server_pid, "911");
        assert_eq!(captured.server_start, "1789109660");
        assert_eq!(captured.session_id, "$7");

        let mut reordered = vec![
            "--quota-dialog".to_owned(),
            "--session-id".to_owned(),
            "$7".to_owned(),
            "--server-start".to_owned(),
            "1789109660".to_owned(),
            "--server-pid".to_owned(),
            "911".to_owned(),
            "--client-pid".to_owned(),
            "4242".to_owned(),
            "--client".to_owned(),
            "/dev/ttys004".to_owned(),
        ];
        assert!(super::parse_quota_dialog(&reordered).is_ok());
        reordered.push("--deadline".to_owned());
        reordered.push("1789109780".to_owned());
        assert!(super::parse_quota_dialog(&reordered).is_err());

        for broken in [
            args()[1..].to_vec(),
            {
                let mut tail = args();
                tail[2] = "not a client!".to_owned();
                tail
            },
            {
                let mut tail = args();
                tail[4] = "4.2.4.2".to_owned();
                tail
            },
            {
                let mut tail = args();
                tail[10] = "not-a-session".to_owned();
                tail
            },
            {
                let mut tail = args();
                tail.extend(["--client".to_owned(), "other".to_owned()]);
                tail
            },
            {
                let mut tail = args();
                tail.pop();
                tail
            },
        ] {
            assert!(super::parse_quota_dialog(&broken).is_err(), "{broken:?}");
        }
    }

    #[test]
    fn client_name_grammar_is_one_predicate_for_both_continuations() {
        assert!(super::is_client_name("/dev/ttys004"));
        for bad in ["", "has space", "semi;colon", "hash#tag", &"x".repeat(129)] {
            assert!(!super::is_client_name(bad), "{bad:?}");
        }
    }
}
