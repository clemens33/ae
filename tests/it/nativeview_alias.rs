//! A native view's grouped session must never become a pane's identity.
//!
//! tmux resolves `-t %pane` (and an untargeted read from inside a pane) to the
//! MOST RECENTLY ACTIVE session holding the pane's window. While a human
//! watches a seat through `_aeview-*`, that is the view session, so every
//! pane-keyed identity read would name it — and a seat's own `send` would be
//! refused as "another ae session". These run against a real private server.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fs;
use std::path::{Path, PathBuf};

use ae::inventory::ServerId;
use ae::meta::Selector;
use ae::tmux::OptionReading;

use super::parity::{Invocation, capture::raw};
use super::phase2::{run_tmux, tmux_present};

/// Kills the private server and removes the scratch even when an assert unwinds.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let out = self.scratch.join("cleanup-out");
        let err = self.scratch.join("cleanup-err");
        let invocation = Invocation::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server");
        let _ = raw::run(&invocation, &self.scratch, &out, &err);
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

/// One stamped real session `real` with a seat pane, on a private socket.
struct Fixture {
    server: ServerId,
    scratch: PathBuf,
    session_id: String,
    created: String,
    pane: String,
    _cleanup: Cleanup,
}

fn tmux(fixture_server: &ServerId, scratch: &Path, words: &[&str]) -> (bool, String) {
    let mut args = ae::tmux::server_args(fixture_server);
    args.extend(words.iter().map(|word| (*word).to_owned()));
    run_tmux(&args, scratch)
}

fn fixture(tag: &str) -> Fixture {
    let scratch = super::cli::OwnedScratch::root("nv", tag).keep();
    assert!(
        tmux_present(&scratch),
        "tmux is not runnable here, so the alias read cannot be proven"
    );
    let socket = scratch.join("t.sock");
    let cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let server = ServerId::Selected(Selector::Socket(socket));
    let (made, printed) = tmux(
        &server,
        &scratch,
        &[
            "new-session",
            "-d",
            "-s",
            "real",
            "-P",
            "-F",
            "#{session_id} | #{session_created} | #{pane_id}",
            "sleep 600",
        ],
    );
    assert!(made, "the real session must start");
    let fields: Vec<String> = printed.trim().split(" | ").map(ToOwned::to_owned).collect();
    assert_eq!(fields.len(), 3, "new-session printed {printed:?}");
    let (session_id, created, pane) = (fields[0].clone(), fields[1].clone(), fields[2].clone());
    let scope = format!("{session_id}:");
    assert!(
        tmux(
            &server,
            &scratch,
            &["set-option", "-t", &scope, "@ae_session_uuid", "U-REAL"]
        )
        .0
    );
    assert!(
        tmux(
            &server,
            &scratch,
            &["set-option", "-p", "-t", &pane, "@ae_slot", "main"]
        )
        .0
    );
    assert!(
        tmux(
            &server,
            &scratch,
            &["set-option", "-p", "-t", &pane, "@ae_agent", "lead"]
        )
        .0
    );
    Fixture {
        server,
        scratch,
        session_id,
        created,
        pane,
        _cleanup: cleanup,
    }
}

/// Open the view group the way the product does, one second later so its
/// activity is strictly newer than the real session's.
fn open_view(fixture: &Fixture, view: &str) {
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let line = ae::theme::view_status_format("F12 or C-] back to ae app");
    let args = ae::tmux::view_group_args(
        &fixture.server,
        &fixture.session_id,
        view,
        "real",
        "lead",
        &line,
    );
    assert!(
        run_tmux(&args, &fixture.scratch).0,
        "the view group must open"
    );
    let (_, first) = tmux(
        &fixture.server,
        &fixture.scratch,
        &[
            "display-message",
            "-p",
            "-t",
            &fixture.pane,
            "#{session_name}",
        ],
    );
    assert_eq!(
        first.trim(),
        view,
        "precondition: bare tmux resolves the pane to the view session"
    );
}

#[test]
fn a_seat_viewed_natively_still_reads_its_real_session() {
    let fixture = fixture("viewer");
    open_view(&fixture, "_aeview-1-1");

    let viewer = ae::transport::observe_viewer(&fixture.server, &fixture.pane)
        .expect("the seat pane answers");
    assert_eq!(viewer.session.as_deref(), Some("real"));
    assert_eq!(viewer.session_uuid, OptionReading::Set("U-REAL".to_owned()));
    assert_eq!(viewer.slot.as_deref(), Some("main"));
    assert_eq!(viewer.agent.as_deref(), Some("lead"));

    let place = ae::transport::observe_pane_place(&fixture.server, &fixture.pane)
        .expect("the seat pane has a place");
    assert_eq!(place, ("real".to_owned(), "0".to_owned()));

    let identity = ae::transport::observe_pane_session_identity(&fixture.server, &fixture.pane)
        .expect("the seat pane has a session identity");
    assert_eq!(identity.id, fixture.session_id);
    assert_eq!(identity.created, fixture.created);
}

#[test]
fn a_view_session_ae_did_not_stamp_is_never_a_pane_s_identity() {
    let fixture = fixture("unstamped");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let target = format!("={}", fixture.session_id);
    assert!(
        tmux(
            &fixture.server,
            &fixture.scratch,
            &["new-session", "-d", "-s", "_aeview-2-2", "-t", &target],
        )
        .0
    );
    assert_unknown(&fixture, &fixture.pane, "unstamped view");
}

/// Overwrite (or, with `None`, unset) the view's recorded real session.
fn stamp_view_of(fixture: &Fixture, view: &str, value: Option<&str>) {
    let scope = format!("={view}:");
    let words: Vec<&str> = match value {
        Some(value) => vec!["set-option", "-t", &scope, ae::tmux::VIEW_OF_OPTION, value],
        None => vec!["set-option", "-u", "-t", &scope, ae::tmux::VIEW_OF_OPTION],
    };
    assert!(tmux(&fixture.server, &fixture.scratch, &words).0);
}

/// Every pane-keyed identity read of `pane` answers nothing at all.
fn assert_unknown(fixture: &Fixture, pane: &str, case: &str) {
    assert_eq!(
        ae::transport::observe_viewer(&fixture.server, pane),
        None,
        "{case}: viewer"
    );
    assert_eq!(
        ae::transport::observe_pane_place(&fixture.server, pane),
        None,
        "{case}: place"
    );
    assert_eq!(
        ae::transport::observe_pane_session_identity(&fixture.server, pane),
        None,
        "{case}: identity"
    );
}

#[test]
fn a_seat_in_a_window_the_real_session_is_not_showing_still_reads_it() {
    let fixture = fixture("noncurrent");
    let scope = format!("{}:", fixture.session_id);
    let (made, printed) = tmux(
        &fixture.server,
        &fixture.scratch,
        &[
            "new-window",
            "-d",
            "-t",
            &scope,
            "-P",
            "-F",
            "#{pane_id}",
            "sleep 600",
        ],
    );
    assert!(made, "a second, non-current window");
    let second = printed.trim().to_owned();
    open_view(&fixture, "_aeview-3-3");
    let (_, current) = tmux(
        &fixture.server,
        &fixture.scratch,
        &["display-message", "-p", "-t", &scope, "#{window_index}"],
    );
    assert_eq!(current.trim(), "0", "precondition: window 1 is not current");

    let viewer = ae::transport::observe_viewer(&fixture.server, &second)
        .expect("the non-current seat answers");
    assert_eq!(viewer.session.as_deref(), Some("real"));
    assert_eq!(viewer.session_uuid, OptionReading::Set("U-REAL".to_owned()));
    assert_eq!(
        ae::transport::observe_pane_place(&fixture.server, &second),
        Some(("real".to_owned(), "1".to_owned()))
    );
    let identity =
        ae::transport::observe_pane_session_identity(&fixture.server, &second).expect("identity");
    assert_eq!(identity.id, fixture.session_id);
}

#[test]
fn a_view_naming_a_session_the_pane_is_not_in_answers_nothing() {
    let fixture = fixture("forged");
    let (made, other) = tmux(
        &fixture.server,
        &fixture.scratch,
        &[
            "new-session",
            "-d",
            "-s",
            "other",
            "-P",
            "-F",
            "#{session_id}",
            "sleep 600",
        ],
    );
    assert!(made, "an unrelated real session");
    open_view(&fixture, "_aeview-4-4");
    stamp_view_of(&fixture, "_aeview-4-4", Some(other.trim()));
    assert_unknown(&fixture, &fixture.pane, "non-member mapping");
}

#[test]
fn a_view_whose_mapping_is_stale_or_malformed_answers_nothing() {
    let fixture = fixture("stale");
    open_view(&fixture, "_aeview-5-5");
    for (value, case) in [
        (Some("$999"), "stale id"),
        (Some("real"), "a name, not an id"),
        (Some("$"), "no digits"),
        (Some(""), "empty"),
        (None, "unset"),
    ] {
        stamp_view_of(&fixture, "_aeview-5-5", value);
        assert_unknown(&fixture, &fixture.pane, case);
    }
}

/// Whether tmux still knows `pane` on the fixture's server.
fn pane_alive(fixture: &Fixture, pane: &str) -> bool {
    tmux(
        &fixture.server,
        &fixture.scratch,
        &["display-message", "-p", "-t", pane, "#{pane_id}"],
    )
    .1
    .trim()
        == pane
}

/// The names the fixture's server lists.
fn sessions(fixture: &Fixture) -> Vec<String> {
    let (_, listed) = tmux(
        &fixture.server,
        &fixture.scratch,
        &["list-sessions", "-F", "#{session_name}"],
    );
    listed.lines().map(ToOwned::to_owned).collect()
}

#[test]
fn stopping_a_viewed_session_by_id_leaves_no_view_holding_its_panes() {
    let fixture = fixture("stop");
    let scope = format!("{}:", fixture.session_id);
    let (_, second) = tmux(
        &fixture.server,
        &fixture.scratch,
        &[
            "new-window",
            "-d",
            "-t",
            &scope,
            "-P",
            "-F",
            "#{pane_id}",
            "sleep 600",
        ],
    );
    let second = second.trim().to_owned();
    let (made, other) = tmux(
        &fixture.server,
        &fixture.scratch,
        &[
            "new-session",
            "-d",
            "-s",
            "other",
            "-P",
            "-F",
            "#{session_id} | #{pane_id}",
            "sleep 600",
        ],
    );
    assert!(made, "a different live session");
    let (other_id, other_pane) = other
        .trim()
        .split_once(" | ")
        .map(|(id, pane)| (id.to_owned(), pane.to_owned()))
        .expect("id and pane");
    open_view(&fixture, "_aeview-6-6");
    // A half-made view: grouped, but never stamped.
    let target = format!("={}", fixture.session_id);
    assert!(
        tmux(
            &fixture.server,
            &fixture.scratch,
            &["new-session", "-d", "-s", "_aeview-8-8", "-t", &target],
        )
        .0
    );
    let line = ae::theme::view_status_format("F12 or C-] back to ae app");
    let foreign = ae::tmux::view_group_args(
        &fixture.server,
        &other_id,
        "_aeview-7-7",
        "other",
        "x",
        &line,
    );
    assert!(
        run_tmux(&foreign, &fixture.scratch).0,
        "a view of the other session"
    );

    assert_eq!(
        ae::transport::stop_views(&fixture.server, &fixture.session_id),
        ae::transport::StopViews::Clear
    );

    let left = sessions(&fixture);
    assert!(!left.contains(&"real".to_owned()), "{left:?}");
    assert!(
        !left.contains(&"_aeview-8-8".to_owned()),
        "untagged view gone: {left:?}"
    );
    assert!(
        !left.contains(&"_aeview-6-6".to_owned()),
        "owned view gone: {left:?}"
    );
    assert!(!pane_alive(&fixture, &fixture.pane), "the seat pane died");
    assert!(
        !pane_alive(&fixture, &second),
        "the non-current window died"
    );
    assert!(left.contains(&"other".to_owned()), "{left:?}");
    assert!(
        left.contains(&"_aeview-7-7".to_owned()),
        "foreign view kept: {left:?}"
    );
    assert!(
        pane_alive(&fixture, &other_pane),
        "the other session's pane lives"
    );
}

#[test]
fn stopping_an_unviewed_session_by_id_is_unchanged() {
    let fixture = fixture("plainstop");
    let (made, _) = tmux(
        &fixture.server,
        &fixture.scratch,
        &["new-session", "-d", "-s", "keep", "sleep 600"],
    );
    assert!(made);
    assert_eq!(
        ae::transport::stop_views(&fixture.server, &fixture.session_id),
        ae::transport::StopViews::Clear
    );
    assert_eq!(sessions(&fixture), vec!["keep".to_owned()]);
    assert!(!pane_alive(&fixture, &fixture.pane));
}

const UUID: &str = "0f0e0d0c-0b0a-4908-8706-050403020100";
const OTHER_UUID: &str = "1f1e1d1c-1b1a-4918-9716-151413121110";

#[test]
fn a_view_left_holding_a_dead_session_is_found_by_its_ae_identity() {
    let fixture = fixture("orphan");
    let (made, other) = tmux(
        &fixture.server,
        &fixture.scratch,
        &[
            "new-session",
            "-d",
            "-s",
            "other",
            "-P",
            "-F",
            "#{session_id} | #{pane_id}",
            "sleep 600",
        ],
    );
    assert!(made);
    let (other_id, other_pane) = other
        .trim()
        .split_once(" | ")
        .map(|(id, pane)| (id.to_owned(), pane.to_owned()))
        .expect("id and pane");
    let line = ae::theme::view_status_format("F12 or C-] back to ae app");
    for (real, view, uuid) in [
        (fixture.session_id.as_str(), "_aeview-9-9", UUID),
        (other_id.as_str(), "_aeview-10-10", OTHER_UUID),
    ] {
        let group = ae::tmux::view_group_args(&fixture.server, real, view, "real", "lead", &line);
        let args = ae::tmux::with_view_uuid(group, view, uuid).expect("a canonical uuid");
        assert!(run_tmux(&args, &fixture.scratch).0, "{view}");
    }
    // The real session dies outside the stop door: only the view keeps its pane.
    let exact = format!("={}", fixture.session_id);
    assert!(
        tmux(
            &fixture.server,
            &fixture.scratch,
            &["kill-session", "-t", &exact]
        )
        .0
    );
    assert!(
        pane_alive(&fixture, &fixture.pane),
        "precondition: the view holds the harness"
    );

    assert_eq!(
        ae::transport::orphan_views(&fixture.server, ""),
        ae::transport::StopViews::Clear,
        "no identity names no view"
    );
    assert!(
        pane_alive(&fixture, &fixture.pane),
        "an empty identity killed nothing"
    );
    assert_eq!(
        ae::transport::orphan_views(&fixture.server, UUID),
        ae::transport::StopViews::Clear
    );
    let left = sessions(&fixture);
    assert!(!left.contains(&"_aeview-9-9".to_owned()), "{left:?}");
    assert!(
        !pane_alive(&fixture, &fixture.pane),
        "the harness pane died with its view"
    );
    assert!(
        left.contains(&"_aeview-10-10".to_owned()),
        "another session's view kept"
    );
    assert!(pane_alive(&fixture, &other_pane));
}

#[test]
fn a_view_is_never_made_without_a_canonical_identity() {
    let line = ae::theme::view_status_format("F12 or C-] back to ae app");
    let server = ServerId::Selected(Selector::Socket(PathBuf::from("/tmp/x.sock")));
    let group = ae::tmux::view_group_args(&server, "$1", "_aeview-1-1", "s", "l", &line);
    assert!(ae::tmux::with_view_uuid(group.clone(), "_aeview-1-1", "").is_none());
    assert!(ae::tmux::with_view_uuid(group.clone(), "_aeview-1-1", "not-a-uuid").is_none());
    let stamped = ae::tmux::with_view_uuid(group, "_aeview-1-1", UUID).expect("stamped");
    let words = stamped.join(" ");
    assert!(words.ends_with(&format!("set-option -t =_aeview-1-1: @ae_view_uuid {UUID}")));
}

#[test]
fn stopping_the_last_session_on_its_server_is_clear_though_the_server_ends() {
    let fixture = fixture("laststop");
    assert_eq!(
        ae::transport::stop_views(&fixture.server, &fixture.session_id),
        ae::transport::StopViews::Clear
    );
    assert!(sessions(&fixture).is_empty());
    assert_eq!(
        ae::transport::verify_session_absent(&fixture.server, "real"),
        ae::tmux::StopProbe::Absent,
        "the server must be proven gone, not merely unreachable"
    );
}

#[test]
fn a_server_proven_gone_holds_no_view_of_anything() {
    let fixture = fixture("deadserver");
    assert!(tmux(&fixture.server, &fixture.scratch, &["kill-server"]).0);
    assert_eq!(
        ae::transport::orphan_views(&fixture.server, UUID),
        ae::transport::StopViews::Clear
    );
    assert_eq!(
        ae::transport::stop_views(&fixture.server, &fixture.session_id),
        ae::transport::StopViews::Clear
    );
}

#[test]
fn a_server_ae_cannot_read_is_never_taken_for_gone() {
    use std::os::unix::fs::PermissionsExt;

    /// Gives the shut directory its search permission back on every way out,
    /// a panic included, so the scratch can be removed.
    struct Reopen<'a>(&'a Path);
    impl Drop for Reopen<'_> {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o700));
        }
    }

    let scratch = super::cli::OwnedScratch::root("nv", "unreadable").keep();
    // A socket path in a directory nobody may search: connecting is denied
    // (EACCES) on every kernel, which says nothing about whether a server is
    // there. A plain file at the path is not such a server on Linux, where
    // connect answers ECONNREFUSED and tmux reports `no server running`.
    let shut = scratch.join("shut");
    fs::create_dir(&shut).expect("shut directory");
    let server = ServerId::Selected(Selector::Socket(shut.join("t.sock")));
    fs::set_permissions(&shut, fs::Permissions::from_mode(0o000)).expect("deny search");
    let reopen = Reopen(&shut);
    let (listed, _) = tmux(&server, &scratch, &["list-sessions"]);
    let diagnostic = fs::read_to_string(scratch.join("stderr")).unwrap_or_default();
    let views = ae::transport::orphan_views(&server, UUID);
    let stop = ae::transport::stop_views(&server, "$0");
    drop(reopen);
    let _ = fs::remove_dir_all(&scratch);
    assert!(
        !listed && ae::tmux::read_failure(&diagnostic) == ae::tmux::Absence::Unreachable,
        "GUARD the fixture is a denied server, not a gone one: {diagnostic:?}"
    );
    assert!(
        diagnostic.contains("Permission denied"),
        "GUARD the denial is EACCES: {diagnostic:?}"
    );
    assert_eq!(views, ae::transport::StopViews::Unreadable);
    assert_eq!(stop, ae::transport::StopViews::Unreadable);
}
