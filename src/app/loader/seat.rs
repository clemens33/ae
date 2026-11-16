//! The panel's seat ask: when the panel opens with no orchestrator running,
//! the reader starts or resumes the canonical seat through the settings
//! control's own discovery and its unattended apply, a waited child of this
//! binary. A renamed role holder or a namesake without the role is never
//! started: the answer names it and how to reach it. The seat's binding stays
//! with the fleet read's proof; this answer only says what the ask did.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::inventory::ServerId;
use crate::orchestrator::ORCHESTRATOR_SESSION;
use crate::settings_menu::{self, Control};
use crate::transport::{self, SeatApplyRun};

/// What one seat ask came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum Said {
    /// The seat was started, resumed or runs already: the first proof of it
    /// binds the panel. The line is why a start or resume failed, `None` when
    /// it ran or had nothing to do.
    Waiting(Option<String>),
    /// No seat starts on this ask, and why: only a reopen asks again.
    Refused(String),
}

/// Ask for the canonical seat in state root `root`: discovery on the
/// `launch` server (the app's own server proves a live holder's), then a
/// start or resume of the seat named `orchestrator` only.
pub(super) fn ask(root: &Path, invoking: Option<&ServerId>, launch: Option<&ServerId>) -> Said {
    let Some(launch) = launch else {
        return Said::Refused("orchestrator not started: no tmux server to start it on".to_owned());
    };
    let invoking = invoking.unwrap_or(launch);
    let until = Instant::now() + Duration::from_secs(settings_menu::STARTING_BOUND_SECS + 1);
    let control = settled(|| settings_menu::discover(root, invoking, launch), until);
    said(control, |resume| {
        let deadline =
            crate::time::Timestamp::now().epoch() + settings_menu::UNATTENDED_WINDOW_SECS;
        ran(transport::run_seat_apply(
            resume,
            deadline,
            settings_menu::UNATTENDED_WAIT,
        ))
    })
}

/// How often an ask reads again a canonical start that has not published.
const STARTING_POLL: Duration = Duration::from_millis(100);

/// The control once no canonical start is between its directory and its
/// meta: `discover` again while it says Starting, until `until`. A start that
/// never publishes ages past the census bound, so discovery itself stops
/// saying Starting; `until` only bounds a clock that does not move.
fn settled(mut discover: impl FnMut() -> Control, until: Instant) -> Control {
    loop {
        let control = discover();
        if control != Control::Starting || Instant::now() >= until {
            return control;
        }
        std::thread::sleep(STARTING_POLL);
    }
}

/// What `control` says to the panel; `apply` starts the seat (`None`) or
/// resumes this identity, and answers why it failed.
pub(super) fn said(control: Control, apply: impl FnOnce(Option<&str>) -> Option<String>) -> Said {
    match control {
        Control::Start => Said::Waiting(apply(None)),
        Control::Resume(target) if target.name == ORCHESTRATOR_SESSION => {
            Said::Waiting(apply(Some(&target.uuid)))
        }
        Control::Pause { target, .. } if target.name == ORCHESTRATOR_SESSION => Said::Waiting(None),
        Control::Resume(target) => Said::Refused(format!(
            "the orchestrator role is held by '{name}', which is stopped; ae {name} resumes it",
            name = target.name
        )),
        Control::Pause { target, .. } => Said::Refused(format!(
            "the orchestrator role is held by '{name}'; ae {name} attaches it",
            name = target.name
        )),
        Control::Starting => Said::Refused(format!(
            "orchestrator not started: its start published no record within {} s",
            settings_menu::STARTING_BOUND_SECS
        )),
        Control::Unavailable(why) => Said::Refused(format!("orchestrator not started: {why}")),
    }
}

/// Why the unattended child failed, from how it ended: its one stdout line,
/// `None` when it exited 0.
pub(super) fn ran(run: SeatApplyRun) -> Option<String> {
    match run {
        SeatApplyRun::Ended { code: Some(0), .. } => None,
        SeatApplyRun::Ended { code, stdout } => {
            let text = String::from_utf8_lossy(&stdout);
            let line = text.lines().map(str::trim).find(|line| !line.is_empty());
            Some(line.map_or_else(
                || match code {
                    Some(code) => format!("orchestrator not started: the start exited {code}"),
                    None => "orchestrator not started: the start was killed".to_owned(),
                },
                str::to_owned,
            ))
        }
        SeatApplyRun::TimedOut => Some(format!(
            "orchestrator not started: the start ran past {} s",
            settings_menu::UNATTENDED_WAIT.as_secs()
        )),
        SeatApplyRun::Failed => {
            Some("orchestrator not started: the start could not run".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_menu::RoleTarget;

    fn target(name: &str) -> RoleTarget {
        RoleTarget {
            name: name.to_owned(),
            uuid: "11111111-2222-4333-8444-555555555555".to_owned(),
            server: ServerId::Ambient,
            pane: "%1".to_owned(),
        }
    }

    #[test]
    fn only_the_canonical_seat_is_started_or_resumed() {
        let mut asked = Vec::new();
        let mut apply = |resume: Option<&str>| {
            asked.push(resume.map(str::to_owned));
            None
        };
        assert_eq!(said(Control::Start, &mut apply), Said::Waiting(None));
        let canonical = target(ORCHESTRATOR_SESSION);
        let uuid = canonical.uuid.clone();
        assert_eq!(
            said(Control::Resume(canonical), &mut apply),
            Said::Waiting(None)
        );
        assert_eq!(asked, [None, Some(uuid)]);
    }

    #[test]
    fn a_live_canonical_seat_waits_for_its_proof_and_starts_nothing() {
        let control = Control::Pause {
            target: target(ORCHESTRATOR_SESSION),
            session_id: "$1".to_owned(),
        };
        let said = said(control, |_| panic!("a live seat is never started"));
        assert_eq!(said, Said::Waiting(None));
    }

    #[test]
    fn a_failed_start_keeps_waiting_with_its_reason() {
        let why = "ae orchestrator: no profile for the seat".to_owned();
        let said = said(Control::Start, |_| Some(why.clone()));
        assert_eq!(said, Said::Waiting(Some(why)));
    }

    #[test]
    fn a_renamed_holder_is_named_with_its_way_in_and_never_started() {
        let never =
            |_: Option<&str>| -> Option<String> { panic!("a renamed holder is never started") };
        let Said::Refused(stopped) = said(Control::Resume(target("router")), never) else {
            panic!("a stopped renamed holder refuses");
        };
        assert!(stopped.contains("'router'") && stopped.contains("ae router resumes it"));
        let live = Control::Pause {
            target: target("router"),
            session_id: "$1".to_owned(),
        };
        let Said::Refused(live) = said(live, never) else {
            panic!("a live renamed holder refuses");
        };
        assert!(live.contains("'router'") && live.contains("ae router attaches it"));
    }

    #[test]
    fn an_unavailable_control_refuses_with_its_reason() {
        let why = "the canonical 'orchestrator' state exists without the orchestrator role";
        let said = said(Control::Unavailable(why.to_owned()), |_| {
            panic!("an unavailable control starts nothing")
        });
        assert_eq!(
            said,
            Said::Refused(format!("orchestrator not started: {why}"))
        );
    }

    #[test]
    fn the_child_line_is_the_reason_and_exit_zero_is_none() {
        let ok = SeatApplyRun::Ended {
            code: Some(0),
            stdout: b"started orchestrator\n".to_vec(),
        };
        assert_eq!(ran(ok), None);
        let refused = SeatApplyRun::Ended {
            code: Some(1),
            stdout: b"\n  ae orchestrator: no profile for the seat  \nmore\n".to_vec(),
        };
        assert_eq!(
            ran(refused).as_deref(),
            Some("ae orchestrator: no profile for the seat")
        );
        let silent = SeatApplyRun::Ended {
            code: Some(3),
            stdout: Vec::new(),
        };
        assert_eq!(
            ran(silent).as_deref(),
            Some("orchestrator not started: the start exited 3")
        );
        let killed = SeatApplyRun::Ended {
            code: None,
            stdout: Vec::new(),
        };
        assert_eq!(
            ran(killed).as_deref(),
            Some("orchestrator not started: the start was killed")
        );
        assert!(ran(SeatApplyRun::TimedOut).is_some_and(|line| line.contains("ran past 90 s")));
        assert_eq!(
            ran(SeatApplyRun::Failed).as_deref(),
            Some("orchestrator not started: the start could not run")
        );
    }
}
