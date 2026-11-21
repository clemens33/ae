//! Bound end pins from nav reviews review-20261120T235608Z-04c6e1ba and
//! review-20261121T002154Z-b7cb151f: a target proved stopped is judged again,
//! read only, after the audit and under the archive holds (absent from its
//! server, and no pane view holding its windows); the archive holds compare
//! the one canonical uuid with the request; an error after the admission
//! still settles the dispatch result; and the dispatcher's plain `rm` and
//! fleet end name the owner's own form.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures own the session records, retained archive and private tmux"
)]

use super::o2guard_p2_spec::uuid_of;
use super::orchrename_rename_spec::{declare, dispatcher, live, refuses, succeeds};
use super::relay_chat_spec::{Out, events, words};
use super::renametell_routing_spec::{B, Rig};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

const TARGET: &str = "endfix-target";

fn archive(rig: &Rig, id: &str) -> PathBuf {
    rig.root.join("archive").join(id)
}

fn all_done(rig: &Rig, target: &str, panes: &[String; 2]) {
    for pane in panes {
        declare(rig, target, pane, "done");
    }
}

fn out_of(output: &std::process::Output) -> Out {
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The target, stopped: its tmux session is gone while `keep` keeps the
/// server answering.
fn stopped_target(rig: &Rig, keep: &str) {
    rig.create(TARGET, B);
    rig.tmux(&["new-session", "-d", "-s", keep, "sh"]);
    rig.tmux(&["kill-session", "-t", &format!("={TARGET}")]);
    assert!(!live(rig, TARGET));
}

/// A grouped view of the target that shares its windows, recorded as a view
/// of its uuid: its tmux session id.
fn view_of_target(rig: &Rig, view: &str) -> String {
    rig.tmux(&["new-session", "-d", "-s", view, "-t", &format!("={TARGET}")]);
    let target = format!("={view}:");
    rig.tmux(&["set-option", "-t", &target, "@ae_view_uuid", B]);
    rig.tmux(&["display-message", "-p", "-t", &target, "#{session_id}"])
        .trim()
        .to_owned()
}

/// A stopped target whose pane view still holds its windows still runs its
/// agents there. The bound end kills nothing by a saved id: it keeps the
/// record, archives nothing, and the view still runs.
#[test]
fn a_stopped_target_whose_pane_view_holds_its_windows_stays_registered() {
    let rig = Rig::new("orf-view");
    rig.create(TARGET, B);
    let id = view_of_target(&rig, "_aeview-orf");
    rig.tmux(&["kill-session", "-t", &format!("={TARGET}")]);
    assert!(!live(&rig, TARGET));
    let out = rig.run(&["end", TARGET, "--keep-history", "--expect-uuid", B], None);
    refuses(&out);
    let text = words(&out);
    assert!(
        text.contains(&format!(
            "pane view session(s) {id} still hold the windows of '{TARGET}'"
        )) && text.contains("Nothing was stopped and nothing was deleted."),
        "{text}"
    );
    assert!(rig.dir(TARGET).is_dir(), "the record stays");
    assert!(!archive(&rig, B).exists(), "nothing is archived");
    assert!(live(&rig, "_aeview-orf"), "the view still runs");
}

/// The control: a stopped target with no view ends.
#[test]
fn a_stopped_target_with_no_view_ends() {
    let rig = Rig::new("orf-noview");
    stopped_target(&rig, "keepalive");
    succeeds(&rig.run(&["end", TARGET, "--keep-history", "--expect-uuid", B], None));
    assert!(!rig.dir(TARGET).exists());
    assert!(archive(&rig, B).is_dir());
}

/// The dispatcher's bound end of a live, all-done target, started while the
/// fixture holds the target's memo lock, so the end waits under the archive's
/// meta hold after its stop. `meddle` changes the stopped record before the
/// hold is released.
fn end_while_the_memo_is_held(rig: &Rig, meddle: impl FnOnce(&Path)) -> (String, Out) {
    let caller = dispatcher(rig);
    let panes = rig.create(TARGET, B);
    all_done(rig, TARGET, &panes);
    let memo_hold = ae::store::lock(&rig.dir(TARGET).join("memo.tsv.lock"), Duration::ZERO)
        .expect("hold the archive memo read");
    let child = rig
        .command(Some(&caller))
        .args(["end", TARGET, "--keep-history", "--expect-uuid", B])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the bound end");
    let until = Instant::now() + Duration::from_secs(30);
    while live(rig, TARGET) {
        assert!(
            Instant::now() < until,
            "the bound end never reached its stop"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    meddle(&rig.dir(TARGET));
    drop(memo_hold);
    let output = child.wait_with_output().expect("bound end result");
    (caller, out_of(&output))
}

/// A second, conflicting uuid row written after the stop: the first row still
/// matches, but the record no longer names exactly the proved uuid, so the
/// end refuses before the archive.
#[test]
fn a_second_uuid_row_written_after_the_stop_keeps_the_target_registered() {
    let rig = Rig::new("orf-drift");
    let other = uuid_of("endfix-drift");
    let (_, out) = end_while_the_memo_is_held(&rig, |dir| {
        let meta = fs::read_to_string(dir.join("meta")).expect("meta");
        fs::write(dir.join("meta"), format!("{meta}session_id={other}\n")).expect("drift");
    });
    refuses(&out);
    assert!(
        words(&out).contains(&format!("no longer names exactly the session {B}")),
        "{}",
        words(&out)
    );
    assert!(rig.dir(TARGET).is_dir(), "the stopped record stays");
    assert!(
        !archive(&rig, B).exists(),
        "the refusal precedes the archive"
    );
    assert!(!archive(&rig, &other).exists());
}

/// A session of the target's name that starts after the stop, while the end
/// waits for the archive holds: the held veto finds it running, the record
/// stays, nothing is archived, and the refusal does not call it stopped.
#[test]
fn a_session_started_after_the_stop_is_not_archived_and_not_called_stopped() {
    let rig = Rig::new("orf-late");
    let (_, out) = end_while_the_memo_is_held(&rig, |_| {
        rig.tmux(&["new-session", "-d", "-s", TARGET, "sh"]);
    });
    refuses(&out);
    let text = words(&out);
    assert!(
        text.contains(&format!("'{TARGET}' runs on its tmux server again"))
            && text.contains("ae cannot prove it is stopped")
            && !text.contains("is stopped and still registered"),
        "{text}"
    );
    assert!(rig.dir(TARGET).is_dir(), "the record stays");
    assert!(!archive(&rig, B).exists(), "the veto precedes the archive");
    assert!(live(&rig, TARGET), "the new session still runs");
}

/// An I/O error after the admission (the held record cannot be read) is
/// still a settled dispatch: the seat's journal gets a non-ok result for the
/// request, and the caller reads the error.
#[test]
fn an_error_after_the_admission_still_settles_the_dispatch_result() {
    let rig = Rig::new("orf-io");
    let (_, out) = end_while_the_memo_is_held(&rig, |dir| {
        fs::set_permissions(dir.join("meta"), fs::Permissions::from_mode(0o000))
            .expect("unreadable meta");
    });
    fs::set_permissions(
        rig.dir(TARGET).join("meta"),
        fs::Permissions::from_mode(0o644),
    )
    .expect("readable again for cleanup");
    refuses(&out);
    assert!(
        words(&out).contains(&format!("could not finish ending '{TARGET}'")),
        "{}",
        words(&out)
    );
    let rows = events(&rig.dir("orchestrator"));
    let request = rows
        .iter()
        .find(|row| {
            row.action == "dispatch-request"
                && row.summary.as_deref() == Some(format!("end {TARGET}").as_str())
        })
        .expect("the dispatch request");
    let result = rows
        .iter()
        .find(|row| row.action == "dispatch-result" && row.reference == request.reference)
        .expect("a settled dispatch result");
    assert_ne!(
        result.summary.as_deref(),
        Some(format!("end {TARGET}: ok").as_str())
    );
    assert!(!archive(&rig, B).exists());
}

/// The dispatcher's plain rm of another session and its fleet end name the
/// owner's own command, never a bound Keep form for them.
#[test]
fn the_dispatchers_plain_rm_and_fleet_end_name_the_owners_form() {
    let rig = Rig::new("orf-owner");
    let caller = dispatcher(&rig);
    rig.create(TARGET, B);
    for (args, owner) in [
        (vec!["rm", TARGET], format!("ae rm {TARGET}")),
        (vec!["end", "all", "-f"], "ae end all -f".to_owned()),
    ] {
        let out = rig.run(&args, Some(&caller));
        refuses(&out);
        let text = words(&out);
        assert!(
            text.contains(&format!(
                "The owner runs, in the owner's own terminal: {owner}."
            )) && !text.contains("--expect-uuid"),
            "{text}"
        );
    }
    assert!(live(&rig, TARGET));
}
