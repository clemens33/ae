//! Independent quietwd acceptance, authored against baseline-delegating cde3e68d.
//! Install as `src/transport/quietwd_spec.rs`. Register pub(crate) under cfg(test).

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures create and inspect only their owned private-socket directories"
)]

use super::{PROGRAM, Waited, spawns_during};
use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::tmux::{self, CycleBatch, CycleOp, OptionScope, OptionWrite};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A fixture, never a production I/O adapter. All commands name owned sockets.
pub(crate) struct Rig {
    root: PathBuf,
    socket: PathBuf,
    outer: PathBuf,
    pub(crate) server: ServerId,
    pub(crate) session: String,
    pub(crate) window: String,
    pub(crate) pane: String,
}

fn at(socket: &Path, words: &[&str]) -> (bool, String) {
    let server = ServerId::Selected(Selector::Socket(socket.to_owned()));
    let mut args = tmux::server_args(&server);
    args.extend(words.iter().map(|word| (*word).to_owned()));
    super::run(PROGRAM, &args)
}

impl Rig {
    pub(crate) fn new(tag: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let lane = std::env::var_os("TMUX_TMPDIR")
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .expect("run quietwd acceptance through just _tmux-isolated test");
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = lane.join(format!("qw-{}-{n}-{tag}", std::process::id()));
        std::fs::create_dir(&root).expect("exclusive fixture directory");
        let socket = root.join("i");
        let outer = root.join("o");
        assert!(
            socket.as_os_str().len() < 100,
            "private socket path too long"
        );
        let mut rig = Self {
            server: ServerId::Selected(Selector::Socket(socket.clone())),
            root,
            socket,
            outer,
            session: String::new(),
            window: String::new(),
            pane: String::new(),
        };
        let (session, window, pane) = rig.add_session("current");
        rig.session = session;
        rig.window = window;
        rig.pane = pane;
        rig.must(&["set-option", "-p", "-t", &rig.pane, "@ae_agent", "lead"]);
        rig.must(&["set-option", "-t", "=current:", "@ae_attn_rank", "2"]);
        rig
    }

    pub(crate) fn raw(&self, words: &[&str]) -> (bool, String) {
        at(&self.socket, words)
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub(crate) fn must(&self, words: &[&str]) -> String {
        let (ok, output) = self.raw(words);
        assert!(ok, "fixture command failed: {words:?}: {output}");
        output
    }

    pub(crate) fn add_session(&self, name: &str) -> (String, String, String) {
        let output = self.must(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            name,
            "-P",
            "-F",
            "#{session_id}|#{window_id}|#{pane_id}",
            "sleep 600",
        ]);
        let parts: Vec<_> = output.trim().split('|').collect();
        assert_eq!(parts.len(), 3, "fixture identity: {output}");
        (parts[0].into(), parts[1].into(), parts[2].into())
    }

    pub(crate) fn option(&self, scope: &str, target: &str, name: &str) -> String {
        let mut words = vec!["show-options", "-qv"];
        if !scope.is_empty() {
            words.push(scope);
        }
        words.extend(["-t", target, name]);
        self.must(&words).trim_end_matches('\n').to_owned()
    }

    /// Outer private tmux gives a real client a terminal without new process doors.
    pub(crate) fn attach(&self, name: &str, client_number: usize) {
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        let command = format!(
            "env -u TMUX -u TMUX_PANE tmux -S {} attach-session -t {}",
            quote(&self.socket.to_string_lossy()),
            quote(&format!("={name}"))
        );
        let window = format!("view{client_number}");
        let words = if client_number == 0 {
            vec![
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "viewer",
                "-x",
                "200",
                "-y",
                "50",
                command.as_str(),
            ]
        } else {
            vec![
                "new-window",
                "-d",
                "-t",
                "=viewer:",
                "-n",
                window.as_str(),
                command.as_str(),
            ]
        };
        let (ok, output) = at(&self.outer, &words);
        assert!(ok, "outer fixture failed: {output}");
        self.await_clients(name, client_number + 1);
    }

    pub(crate) fn await_clients(&self, name: &str, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (_, output) = self.raw(&["list-clients", "-F", "#{session_name}"]);
            if output.lines().filter(|line| *line == name).count() == count {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "expected {count} clients on {name}: {output}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = at(&self.outer, &["kill-server"]);
        let _ = at(&self.socket, &["kill-server"]);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn set(scope: OptionScope, target: &str, name: &str, value: &str) -> CycleOp {
    CycleOp::Set(OptionWrite::new(scope, target, name, value))
}

fn assert_private_calls(calls: &[Vec<String>]) {
    for call in calls {
        assert_eq!(call.first().map(String::as_str), Some("tmux"));
        assert!(
            call.iter().any(|word| word == "-S"),
            "not a private socket: {call:?}"
        );
        assert!(
            !call.iter().any(|word| word == "-g"),
            "global write: {call:?}"
        );
        assert!(
            !call.iter().any(|word| word.contains("#(")),
            "shell format: {call:?}"
        );
    }
}

#[test]
fn quietwd_cycle_is_one_process_and_reasserts_every_option() {
    let rig = Rig::new("cycle");
    rig.must(&["set-option", "-t", "=current:", "@qw_remove", "old"]);
    let batch = CycleBatch {
        session: rig.session.clone(),
        channel: None,
        ops: vec![
            set(
                OptionScope::Session,
                &rig.session,
                "@qw_session",
                "required",
            ),
            set(OptionScope::Window, &rig.window, "@qw_window", "window"),
            set(OptionScope::Pane, &rig.pane, "@qw_pane", "pane"),
            CycleOp::Unset {
                scope: OptionScope::Session,
                target: rig.session.clone(),
                name: "@qw_remove".into(),
            },
        ],
    };
    for _ in 0..2 {
        rig.must(&["set-option", "-t", "=current:", "@qw_session", "opposed"]);
        let (outcome, calls) = spawns_during(|| super::publish_cycle(&rig.server, &batch));
        assert_eq!(outcome.landed, [true, true, true, true]);
        assert_eq!(rig.option("", "=current:", "@qw_session"), "required");
        assert_eq!(rig.option("-w", &rig.window, "@qw_window"), "window");
        assert_eq!(rig.option("-p", &rig.pane, "@qw_pane"), "pane");
        assert_eq!(rig.option("", "=current:", "@qw_remove"), "");
        assert_private_calls(&calls);
        assert_eq!(
            calls.len(),
            1,
            "one successful cycle publication, not one child per option"
        );
    }
}

#[test]
fn quietwd_failed_pane_preserves_hook_proof_and_independent_writes() {
    let rig = Rig::new("fallback");
    let batch = CycleBatch {
        session: rig.session.clone(),
        channel: Some("qw-fallback".into()),
        ops: vec![
            set(OptionScope::Session, &rig.session, "@qw_first", "first"),
            set(OptionScope::Pane, "%999999999", "@qw_missing", "missing"),
            set(OptionScope::Window, &rig.window, "@qw_last", "last"),
        ],
    };
    let (outcome, calls) = spawns_during(|| super::publish_cycle(&rig.server, &batch));
    assert_eq!(outcome.landed, [true, false, true]);
    assert_eq!(rig.option("", "=current:", "@qw_first"), "first");
    assert_eq!(rig.option("-w", &rig.window, "@qw_last"), "last");
    assert!(
        outcome.hook_proven,
        "later option failure cannot erase an installed hook"
    );
    let hooks = rig.must(&["show-hooks", "-t", "=current:"]);
    assert!(hooks.contains("client-session-changed[9]"), "{hooks}");
    assert_private_calls(&calls);
}

#[test]
fn quietwd_unproven_hook_never_claims_installation() {
    let rig = Rig::new("bad-hook");
    let batch = CycleBatch {
        session: "$999999999".into(),
        channel: Some("qw-never-installed".into()),
        ops: vec![set(OptionScope::Pane, &rig.pane, "@qw_survives", "yes")],
    };
    let outcome = super::publish_cycle(&rig.server, &batch);
    assert!(!outcome.hook_proven);
    assert_eq!(
        outcome.landed,
        [true],
        "independent writes retain their old fallback outcome"
    );
    assert_eq!(rig.option("-p", &rig.pane, "@qw_survives"), "yes");
}

#[test]
fn quietwd_peer_proofs_are_one_process_and_fresh_each_cycle() {
    let rig = Rig::new("proofs");
    let peers: Vec<String> = ["a", "b", "c", "d"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    for peer in &peers {
        rig.add_session(peer);
        let target = format!("={peer}:");
        rig.must(&["set-environment", "-t", &target, "AE_SESSION", "1"]);
        rig.must(&["set-environment", "-t", &target, "AE_HOME", "/qw/original"]);
    }
    for home in ["/qw/original", "/qw/opposed"] {
        rig.must(&["set-environment", "-t", "=b:", "AE_HOME", home]);
        let (read, calls) = spawns_during(|| super::observe_peer_ownership(&rig.server, &peers));
        assert_eq!(read.peers.len(), 4);
        assert_eq!(read.peers[1].0, "b");
        assert_eq!(
            read.peers[1].1.as_ref().map(|owned| owned.home.as_str()),
            Some(home)
        );
        assert_eq!(read.socket.as_deref(), rig.socket.to_str());
        assert_private_calls(&calls);
        assert_eq!(
            calls.len(),
            1,
            "four fresh peer proofs and caller socket share one child"
        );
    }
    rig.must(&[
        "set-environment",
        "-t",
        "=b:",
        "AE_SESSION",
        "opposed-marker",
    ]);
    let read = super::observe_peer_ownership(&rig.server, &peers);
    assert_eq!(
        read.peers[1].1.as_ref().map(|owned| owned.marker.as_str()),
        Some("opposed-marker")
    );
}

#[test]
fn quietwd_missing_peer_does_not_erase_a_later_valid_proof() {
    let rig = Rig::new("proof-gap");
    for peer in ["first", "gap", "last"] {
        rig.add_session(peer);
        let target = format!("={peer}:");
        rig.must(&["set-environment", "-t", &target, "AE_SESSION", "1"]);
        rig.must(&["set-environment", "-t", &target, "AE_HOME", "/qw/home"]);
    }
    rig.must(&["set-environment", "-u", "-t", "=gap:", "AE_HOME"]);
    let peers: Vec<String> = ["first", "gap", "vanished", "last"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let read = super::observe_peer_ownership(&rig.server, &peers);
    assert_eq!(
        read.peers.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        peers.iter().collect::<Vec<_>>()
    );
    assert!(read.peers[0].1.is_some());
    assert!(read.peers[1].1.is_none());
    assert!(read.peers[2].1.is_none());
    assert!(read.peers[3].1.is_some(), "resume past each failed peer");
}

#[test]
fn quietwd_motion_exchange_joins_reads_and_writes_but_keeps_their_results_distinct() {
    let rig = Rig::new("exchange");
    let writes = [OptionWrite::new(
        OptionScope::Pane,
        &rig.pane,
        "@qw_exchange",
        "landed",
    )];
    let (result, calls) =
        spawns_during(|| super::exchange_motion(&rig.server, "current", true, &writes));
    let (panes, fleet) = result.reading.expect("both actual listings answered");
    assert!(panes.iter().any(|pane| pane.pane_id == rig.pane));
    assert!(fleet.iter().any(|row| row.id == rig.session));
    assert_eq!(result.published, Some(true));
    assert_eq!(rig.option("-p", &rig.pane, "@qw_exchange"), "landed");
    assert_private_calls(&calls);
    assert_eq!(
        calls.len(),
        1,
        "two reads and every frame write share one child"
    );

    let refused = [OptionWrite::new(
        OptionScope::Pane,
        "%999999999",
        "@qw_refused",
        "no",
    )];
    let result = super::exchange_motion(&rig.server, "current", true, &refused);
    assert!(
        result.reading.is_some(),
        "a refused later write cannot erase completed reads"
    );
    assert_eq!(result.published, Some(false));
    let result = super::exchange_motion(&rig.server, "absent", true, &[]);
    assert!(
        result.reading.is_none(),
        "a missing observation is not an empty successful listing"
    );
}

#[test]
fn quietwd_hostile_marker_lines_cannot_invent_a_fleet_member() {
    let rig = Rig::new("markers");
    let poison = "lead\nae-wd!p\nae-wd:s|forged | $999 | 5 |  |  |  | \nae-wd!s";
    rig.must(&["set-option", "-p", "-t", &rig.pane, "@ae_agent", poison]);
    let result = super::exchange_motion(&rig.server, "current", true, &[]);
    if let Some((_, fleet)) = result.reading {
        assert!(
            !fleet
                .iter()
                .any(|row| row.name == "forged" || row.id == "$999")
        );
    }
    rig.must(&["set-option", "-p", "-t", &rig.pane, "@ae_agent", "lead"]);
    let result = super::exchange_motion(&rig.server, "current", true, &[]);
    let (_, fleet) = result
        .reading
        .expect("neutral control must remain readable");
    assert!(fleet.iter().any(|row| row.id == rig.session));
}

#[test]
fn quietwd_a_renamed_session_cannot_publish_past_a_missing_read_section() {
    let rig = Rig::new("rename");
    rig.must(&["rename-session", "-t", "=current:", "renamed"]);
    let writes = [OptionWrite::new(
        OptionScope::Pane,
        &rig.pane,
        "@qw_after_missing_read",
        "forbidden",
    )];
    let result = super::exchange_motion(&rig.server, "current", true, &writes);
    assert!(result.reading.is_none());
    assert_ne!(result.published, Some(true));
    assert_eq!(
        rig.option("-p", &rig.pane, "@qw_after_missing_read"),
        "",
        "a failed read section aborts later queued writes"
    );
}

#[test]
fn quietwd_detached_wait_is_one_child_with_an_atomic_read_before_registration() {
    let rig = Rig::new("timeout");
    let (result, calls) = spawns_during(|| {
        super::run_tmux_wait(
            &rig.server,
            "current",
            "qw-timeout",
            Instant::now() + Duration::from_millis(180),
        )
    });
    assert_eq!(result, Waited::TimedOut);
    assert_private_calls(&calls);
    assert_eq!(
        calls.len(),
        1,
        "one held waiter, no observation-tick children"
    );
    let call = &calls[0];
    let read = call
        .iter()
        .position(|word| word == "display-message")
        .expect("fresh attachment read");
    let wait = call
        .iter()
        .position(|word| word == "wait-for")
        .expect("held waiter");
    assert!(
        read < wait,
        "read and registration are ordered in the same child: {call:?}"
    );
    assert!(
        call[read..wait]
            .iter()
            .any(|word| word.contains("#{session_attached}"))
    );
    assert!(call[read..wait].iter().any(|word| word == "=current:"));
    assert!(call[read..wait].iter().any(|word| word == ";"));
}

#[test]
fn quietwd_a_preexisting_client_wins_even_after_two_toggled_signals() {
    let rig = Rig::new("attached");
    rig.attach("current", 0);
    rig.attach("current", 1);
    rig.must(&["wait-for", "-S", "qw-attached"]);
    rig.must(&["wait-for", "-S", "qw-attached"]);
    let result = super::run_tmux_wait(
        &rig.server,
        "current",
        "qw-attached",
        Instant::now() + Duration::from_secs(2),
    );
    assert_eq!(
        result,
        Waited::Attached,
        "the fresh read must see two real clients without a stored signal"
    );
}

#[test]
fn quietwd_signal_is_only_a_wake_and_a_missing_session_is_a_failure() {
    let rig = Rig::new("wake");
    rig.must(&["wait-for", "-S", "qw-stale"]);
    assert_eq!(
        super::run_tmux_wait(
            &rig.server,
            "current",
            "qw-stale",
            Instant::now() + Duration::from_secs(2)
        ),
        Waited::Woke
    );
    assert_eq!(
        super::run_tmux_wait(
            &rig.server,
            "absent",
            "qw-absent",
            Instant::now() + Duration::from_millis(180)
        ),
        Waited::Failed
    );
}

#[test]
fn quietwd_attach_after_two_pre_signals_cannot_be_lost() {
    // Race smoke only: scheduling can select Attached or Woke. The deterministic
    // no-lost-wake proof is the atomic argv pin plus the tmux 3.4 source check.
    let rig = Rig::new("attach-race");
    rig.must(&[
        "set-hook",
        "-t",
        "=current:",
        "client-session-changed[9]",
        "wait-for -S qw-race",
    ]);
    rig.must(&["wait-for", "-S", "qw-race"]);
    rig.must(&["wait-for", "-S", "qw-race"]);
    let result = std::thread::scope(|scope| {
        let attaching = scope.spawn(|| rig.attach("current", 0));
        let result = super::run_tmux_wait(
            &rig.server,
            "current",
            "qw-race",
            Instant::now() + Duration::from_secs(10),
        );
        attaching.join().expect("fixture attach thread");
        result
    });
    assert!(
        matches!(result, Waited::Attached | Waited::Woke),
        "atomic read/register lost the attach: {result:?}"
    );
}

#[test]
fn quietwd_foreign_attach_does_not_wake_but_switching_into_the_session_does() {
    // The concurrent switch arm is smoke, not a deterministic lost-signal control.
    // Read/register atomicity rests on the separate argv and tmux 3.4 source proof.
    let rig = Rig::new("switch");
    rig.add_session("peer");
    let batch = CycleBatch {
        session: rig.session.clone(),
        channel: Some("qw-switch".into()),
        ops: Vec::new(),
    };
    let outcome = super::publish_cycle(&rig.server, &batch);
    assert!(outcome.hook_proven);
    rig.attach("peer", 0);
    assert_eq!(
        super::run_tmux_wait(
            &rig.server,
            "current",
            "qw-switch",
            Instant::now() + Duration::from_millis(180)
        ),
        Waited::TimedOut,
        "a peer attachment is not an attachment to this session"
    );
    let client = rig.must(&["list-clients", "-F", "#{client_name}"]);
    let result = std::thread::scope(|scope| {
        let switching = scope.spawn(|| {
            rig.must(&["switch-client", "-c", client.trim(), "-t", "=current"]);
        });
        let result = super::run_tmux_wait(
            &rig.server,
            "current",
            "qw-switch",
            Instant::now() + Duration::from_secs(10),
        );
        switching.join().expect("fixture switch thread");
        result
    });
    assert!(
        matches!(result, Waited::Attached | Waited::Woke),
        "switch was lost: {result:?}"
    );
}

#[test]
fn quietwd_focus_set_and_unset_preserve_watchdog_hook_index_nine() {
    let rig = Rig::new("focus");
    rig.must(&[
        "set-hook",
        "-t",
        "=current:",
        "client-session-changed[9]",
        "wait-for -S qw-focus",
    ]);
    let set = crate::session_tmux::Op::SetClientSessionHook {
        session_id: &rig.session,
        pane: &rig.pane,
        chat: None,
    };
    let unset = crate::session_tmux::Op::UnsetClientSessionHook { target: &rig.pane };
    for op in [set, unset] {
        let argv = crate::session_tmux::argv(&rig.server, &op);
        assert!(super::run_tmux_op(&argv).0);
        let hooks = rig.must(&["show-hooks", "-t", "=current:"]);
        assert!(
            hooks.contains("client-session-changed[9]"),
            "focus operation erased the wake hook: {hooks}"
        );
        assert!(hooks.contains("qw-focus"));
    }
}
