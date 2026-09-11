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
    Menu {
        title: "ae settings".to_owned(),
        title_style: crate::theme::menu_title_style(palette),
        items: vec![
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
        ],
    }
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
    if client.is_empty()
        || client.len() > 128
        || !client
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._/:+-".contains(&byte))
    {
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
}
