mod effort_resume_spec {
    use super::super::meta_document;
    use super::resume_parts;

    #[test]
    fn effort_resume_rebuild_keeps_a_single_applied_claim_verbatim() {
        const VALID: &str = "v1 applied slot:main base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:lead tool:claude launch:tok-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:fable5 client:- model:opus win:weekly/~/10080/1700000000/1700000100";
        let row = format!("effort.main={VALID}\n");
        let prior = format!("seat.main=lead\nprofile.main=fable5\n{row}");
        let (root, _, env, shape, launching, watchdog) = resume_parts(&prior);
        let rebuilt =
            meta_document(&env, &shape, &launching, watchdog, None).expect("retained document");
        assert!(
            rebuilt.contains(&row),
            "valid applied row survives a whole rebuild"
        );
        assert!(matches!(
            crate::effort::stored(rebuilt.as_bytes(), "main"),
            crate::effort::Stored::Present(_)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn effort_resume_rebuild_carries_raw_claims_only_for_retained_launching_slots() {
        let rows = "effort.main=not-a-record\neffort.main\neffort.main=duplicate\r\n";
        let prior =
            format!("seat.main=lead\nprofile.main=fable5\n{rows}effort.spawned.0=foreign\n");
        let (root, _, env, mut shape, launching, watchdog) = resume_parts(&prior);
        let rebuilt =
            meta_document(&env, &shape, &launching, watchdog, None).expect("resume document");
        let carried: Vec<_> = rebuilt
            .split_terminator('\n')
            .filter(|line| line.split('=').next() == Some("effort.main"))
            .collect();
        assert_eq!(
            carried,
            [
                "effort.main=not-a-record",
                "effort.main",
                "effort.main=duplicate\r"
            ]
        );
        assert!(
            !rebuilt.contains("effort.spawned.0"),
            "only retained launching slots carry"
        );
        shape.resuming = false;
        let fresh =
            meta_document(&env, &shape, &launching, watchdog, None).expect("fresh document");
        assert!(
            !fresh.contains("effort."),
            "a new conversation inherits no override"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn observed_resume_rebuild_carries_a_damaged_record_byte_for_byte() {
        let rows = "observed_model.main=Opus 5.5\nobserved_model_pin.main=fable\nobserved_effort.main=high\nobserved_effort.main\nobserved_effort_pin.main=xhigh\nobserved_effort_pin.main=max\r\n";
        let prior = format!("seat.main=lead\nprofile.main=fable5\n{rows}launched.main=main:L1:7\n");
        let (root, dir, env, shape, launching, watchdog) = resume_parts(&prior);
        let rebuilt =
            meta_document(&env, &shape, &launching, watchdog, None).expect("resume document");
        let carried: String = rebuilt
            .split_inclusive('\n')
            .filter(|line| line.starts_with("observed_"))
            .collect();
        assert_eq!(carried, rows, "parse doubt never repairs a record");
        let damaged = crate::meta::observed_pair(rebuilt.as_bytes(), "main");
        assert_eq!(damaged, crate::meta::Rows::Damaged("duplicate"));
        assert!(
            !rebuilt.contains("launched."),
            "the baseline is never carried"
        );
        // A value that is not UTF-8 is carried lossy, and stays damaged.
        let hostile = b"seat.main=lead\nobserved_model.main=\xff\nobserved_model_pin.main=x\n";
        std::fs::write(dir.join("meta"), hostile).expect("hostile raw meta");
        let rebuilt = meta_document(&env, &shape, &launching, watchdog, None).expect("resume");
        let damaged = crate::meta::observed_pair(rebuilt.as_bytes(), "main");
        assert_eq!(damaged, crate::meta::Rows::Damaged("invalid"), "{rebuilt}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn effort_resume_rebuild_keeps_non_utf8_claim_damaged_instead_of_absent() {
        let (root, dir, env, shape, launching, watchdog) =
            resume_parts("seat.main=lead\nprofile.main=fable5\n");
        std::fs::write(
            dir.join("meta"),
            b"seat.main=lead\nprofile.main=fable5\neffort.main=\xff\n",
        )
        .expect("hostile raw meta");
        let rebuilt =
            meta_document(&env, &shape, &launching, watchdog, None).expect("resume document");
        assert!(
            rebuilt.contains("effort.main=\u{fffd}\n"),
            "named lossy text carry"
        );
        assert_eq!(
            crate::effort::stored(rebuilt.as_bytes(), "main"),
            crate::effort::Stored::Damaged(crate::effort::Damage::Encoding)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
use super::{
    AttachAction, EVENTS_KEEP, EXIT_USAGE, ExpectedLaunch, ExpectedState, Mode, Plan, Relaunch,
    ToolKind, attach_action, carries_own_proof, check_launch_kind, check_resume_target,
    check_stopped_near_miss, expected_launch_argv, launch_token, launch_turn_is_pasted,
    layout_argvs, lead_pair_policy_argvs, parse_plan, parse_plan_with_attach, relaunch,
    replace_watchdog_registration, server_attach_hint, trim_events,
};
use crate::inventory::ServerId;
use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

fn layout_words(layout: &str, panes: &[String], workers: usize) -> Vec<Vec<String>> {
    layout_argvs(&ServerId::Ambient, layout, "named", panes, workers)
        .iter()
        .map(|command| command.as_args().to_vec())
        .collect()
}

fn pair_policy_words(layout: &str, main_pane: &str) -> Vec<Vec<String>> {
    lead_pair_policy_argvs(&ServerId::Ambient, layout, main_pane)
        .iter()
        .map(|command| command.as_args().to_vec())
        .collect()
}

/// PIN (R10): a client entering the session is sent to a chat only when
/// one stamped with the session's own uuid sits in its FIRST window; a
/// chat anywhere else, a foreign stamp or a non-canonical uuid keeps the
/// lead hook. A dead one still counts: the hook falls back when it fires.
#[test]
fn only_a_chat_in_the_first_window_is_where_a_client_lands() {
    let uuid = "0e3c0a4e-7f7e-4c8e-9a3b-2d1f00000001";
    let pane =
        |id: &str, window: u32, index: u32, console: Option<&str>, dead| crate::tmux::WindowPane {
            pane_id: id.to_owned(),
            window_id: format!("@{window}"),
            theme: String::new(),
            reader_src: None,
            console: console.map(str::to_owned),
            dead,
            window_index: window,
            pane_index: index,
            agent: None,
        };
    let lead = pane("%1", 1, 0, None, false);
    let landing = |panes: &[crate::tmux::WindowPane], uuid: &str| {
        super::window_zero_chat(panes, uuid).cloned()
    };
    let chat = |id: &str, window, stamp, dead| pane(id, window, 0, Some(stamp), dead);
    for (dead, want) in [(false, "%4"), (true, "%4")] {
        let panes = [lead.clone(), chat("%4", 0, uuid, dead)];
        assert_eq!(landing(&panes, uuid).as_deref(), Some(want), "dead {dead}");
    }
    let cases = [
        vec![lead.clone()],
        vec![lead.clone(), chat("%4", 2, uuid, false)],
        vec![lead.clone(), chat("%4", 0, "other", false)],
    ];
    for panes in cases {
        assert_eq!(landing(&panes, uuid), None, "{panes:?}");
    }
    let panes = [lead.clone(), chat("%4", 0, "x", false)];
    for stamp in ["", "x", "0E3C0A4E-7F7E-4C8E-9A3B-2D1F00000001"] {
        assert_eq!(landing(&panes, stamp), None, "{stamp:?}");
    }
    let split = [
        lead,
        pane("%6", 0, 1, Some(uuid), false),
        pane("%5", 0, 0, None, false),
        pane("%4", 0, 2, Some(uuid), false),
    ];
    assert_eq!(landing(&split, uuid).as_deref(), Some("%6"));
}

/// A saved session is reattached while it runs and never rebuilt; an
/// unanswered probe is never read as absence; a stopped one is rebuilt
/// where it was only for an in-place resume (picker, settings, restore)
/// of a record that names its server, and on the asking server otherwise.
#[test]
fn a_saved_session_is_placed_by_its_probe_and_resume_kind() {
    use super::{Destination, Placement, placement};
    use crate::tmux::StopProbe;
    for in_place in [false, true] {
        for legacy in [false, true] {
            let case = format!("in_place {in_place} legacy {legacy}");
            assert_eq!(
                placement(&StopProbe::Present, in_place, legacy),
                Placement::Running,
                "{case}"
            );
            assert_eq!(
                placement(&StopProbe::Unknown, in_place, legacy),
                Placement::Unproven,
                "{case}"
            );
            let destination = if in_place && !legacy {
                Destination::Recorded
            } else {
                Destination::Proposed
            };
            assert_eq!(
                placement(&StopProbe::Absent, in_place, legacy),
                Placement::Stopped(destination),
                "{case}"
            );
        }
    }
}

/// A record names one server, names none (legacy: the historical default
/// is probed), or cannot be resolved and names no server at all.
#[test]
fn a_record_names_its_probe_server_or_none() {
    use super::RecordedServer;
    use crate::inventory::ServerId;
    use crate::meta::{Selector, ServerSelector};
    let selector = Selector::Name("ae-dev".to_owned());
    assert_eq!(
        RecordedServer::of(ServerSelector::Positive(selector.clone())),
        Some(RecordedServer {
            server: ServerId::Selected(selector),
            legacy: false
        })
    );
    assert_eq!(
        RecordedServer::of(ServerSelector::Missing),
        Some(RecordedServer {
            server: crate::doors::historical_server(),
            legacy: true
        })
    );
    assert_eq!(RecordedServer::of(ServerSelector::Ambiguous), None);
}

/// META-WINS for a watchdog cadence: the recorded row beats the config and
/// the config is not even read; without a row the config, then the
/// default; and whatever wins must be unsigned seconds.
#[test]
fn a_recorded_cadence_wins_without_reading_the_config() {
    use super::recorded_wins_seconds;
    let unread = || -> Result<Option<String>, String> {
        panic!("the configured value was read although a row is recorded")
    };
    let recorded = |value: &str| Some(value.to_owned());
    assert_eq!(
        recorded_wins_seconds(recorded("45"), unread, 300),
        Ok("45".to_owned())
    );
    assert_eq!(
        recorded_wins_seconds(recorded("0"), unread, 300),
        Ok("0".to_owned())
    );
    for bad in ["", "-1", "1.5", "ten", " 45"] {
        assert_eq!(
            recorded_wins_seconds(recorded(bad), unread, 300),
            Err(bad.to_owned()),
            "{bad:?}"
        );
    }
    assert_eq!(
        recorded_wins_seconds(None, || Ok(Some("90".to_owned())), 300),
        Ok("90".to_owned())
    );
    assert_eq!(
        recorded_wins_seconds(None, || Ok(None), 300),
        Ok("300".to_owned())
    );
    assert_eq!(
        recorded_wins_seconds(None, || Err("broken".to_owned()), 300),
        Err("broken".to_owned())
    );
    assert_eq!(
        recorded_wins_seconds(None, || Ok(Some("soon".to_owned())), 300),
        Err("soon".to_owned())
    );
    let mut reads = 0;
    let counted = || {
        reads += 1;
        Ok(None)
    };
    assert_eq!(recorded_wins_seconds(None, counted, 7), Ok("7".to_owned()));
    assert_eq!(
        reads, 1,
        "an unrecorded cadence reads the config exactly once"
    );
}

/// The mode: the flag, then the recorded one, then config, then local.
#[test]
fn the_flag_beats_the_record_beats_the_config_beats_local() {
    use super::{Mode, launch_mode};
    for configured in ["local", "git", "full", ""] {
        for recorded in [None, Some(Mode::Local), Some(Mode::Git), Some(Mode::Full)] {
            for flag in [Mode::Local, Mode::Git, Mode::Full] {
                assert_eq!(launch_mode(Some(flag), recorded, configured), flag);
            }
            if let Some(recorded) = recorded {
                assert_eq!(launch_mode(None, Some(recorded), configured), recorded);
            }
        }
    }
    assert_eq!(launch_mode(None, None, "git"), Mode::Git);
    assert_eq!(launch_mode(None, None, "full"), Mode::Full);
    assert_eq!(
        launch_mode(None, None, "worktree"),
        Mode::Git,
        "the documented spelling"
    );
    assert_eq!(launch_mode(None, None, "local"), Mode::Local);
    for unknown in ["", "Git", " git", "nonsense", "Worktree"] {
        assert_eq!(launch_mode(None, None, unknown), Mode::Local, "{unknown:?}");
    }
}

/// An unknown configured `copy` warns once and names the value and the
/// accepted spellings; a known spelling or an unset key earns nothing.
#[test]
fn an_unknown_copy_warns_and_a_known_one_does_not() {
    use super::unknown_copy_note;
    for known in ["", "local", "git", "worktree", "full"] {
        assert_eq!(unknown_copy_note(known), None, "{known:?}");
    }
    assert_eq!(
        unknown_copy_note("nonsense").as_deref(),
        Some(
            "Note: [workspace] copy = \"nonsense\" is not a working-copy mode; \
             use local, worktree (or git) or full. The launch uses local."
        )
    );
}

/// A meta agent's session and the orchestrator seat never open the chat
/// and never read its setting, so a malformed one earns no note.
#[test]
fn a_suppressed_chat_never_reads_its_setting() {
    use super::chat_opens;
    for (meta_agent, orchestrator_seat) in [(true, false), (false, true), (true, true)] {
        let unread =
            || -> crate::config::ChatWindow { panic!("a suppressed chat read its setting") };
        assert_eq!(
            chat_opens(meta_agent, orchestrator_seat, unread),
            (false, None),
            "meta={meta_agent}, seat={orchestrator_seat}"
        );
    }
}

/// Otherwise the setting is read exactly once: on and app open the chat,
/// off keeps it closed, and a malformed one opens it and is named.
#[test]
fn an_unsuppressed_chat_reads_its_setting_once() {
    use super::chat_opens;
    use crate::config::ChatWindow;
    for (setting, expected) in [
        (ChatWindow::On, (true, None)),
        (ChatWindow::App, (true, None)),
        (ChatWindow::Off, (false, None)),
        (
            ChatWindow::Unusable("\"x\" is not on".to_owned()),
            (true, Some("\"x\" is not on".to_owned())),
        ),
    ] {
        let mut reads = 0;
        let read = || {
            reads += 1;
            setting.clone()
        };
        assert_eq!(chat_opens(false, false, read), expected, "{setting:?}");
        assert_eq!(reads, 1, "{setting:?}");
    }
}

/// A non-empty recorded layout wins; otherwise the configured one, and
/// `vertical` when nothing is configured.
#[test]
fn a_recorded_layout_beats_the_config_beats_vertical() {
    use super::session_layout;
    for configured in ["", "horizontal", "lead-solo"] {
        assert_eq!(
            session_layout(configured.to_owned(), Some("lead-pair".to_owned())),
            "lead-pair"
        );
        let fallback = if configured.is_empty() {
            "vertical"
        } else {
            configured
        };
        for saved in [None, Some(String::new())] {
            assert_eq!(
                session_layout(configured.to_owned(), saved.clone()),
                fallback,
                "{configured:?} {saved:?}"
            );
        }
    }
}

/// A restore whose record was gone before the lifecycle lock skips with
/// that reason and never asks the guard; a held lock over a present record
/// asks the guard exactly once and answers its verdict unchanged.
#[test]
fn a_restore_skips_a_record_gone_before_the_lock_without_asking_the_guard() {
    use super::restore_skip;
    use std::cell::Cell;
    let gone = Some("saved record gone before the lifecycle lock");
    for (held, meta_present) in [(false, true), (true, false), (false, false)] {
        let asked = Cell::new(0);
        let verdict = restore_skip(held, meta_present, || {
            asked.set(asked.get() + 1);
            None
        });
        assert_eq!(verdict, gone, "held={held} meta_present={meta_present}");
        assert_eq!(asked.get(), 0, "held={held} meta_present={meta_present}");
    }
    for guard in [None, Some("launched again since the scan")] {
        let asked = Cell::new(0);
        let verdict = restore_skip(true, true, || {
            asked.set(asked.get() + 1);
            guard
        });
        assert_eq!(verdict, guard, "{guard:?}");
        assert_eq!(asked.get(), 1, "{guard:?}");
    }
}

#[test]
fn a_rebuild_carries_or_clears_but_never_mints() {
    // The rebuild's half of the boundary — `_run` mints at its
    // own pre-exec, the rebuild only carries the stored id or clears to
    // the honest unknown.
    use crate::tool::ToolKind;
    assert_eq!(
        super::launch_session_id(Some("u-9".to_owned()), true, ToolKind::Claude),
        "u-9",
        "a stored id is carried"
    );
    for tool in [ToolKind::Claude, ToolKind::Grok, ToolKind::Agy] {
        assert_eq!(
            super::launch_session_id(None, true, tool),
            crate::launch::PENDING,
            "a resume without a stored id clears, never mints: {}",
            tool.as_str()
        );
    }
    for tool in [ToolKind::Claude, ToolKind::Grok] {
        let minted = super::launch_session_id(None, false, tool);
        assert_eq!(minted.len(), 36, "a create mints: {}", tool.as_str());
    }
    assert_eq!(
        super::launch_session_id(None, false, ToolKind::Agy),
        crate::launch::PENDING,
        "a create without a launch id stays pending"
    );
}

#[test]
fn normal_expected_launch_argv_cannot_enable_the_debug_pre_lock_marker() {
    let preamble = crate::entry::Preamble::default();
    let user = ["orchestrator".to_owned()];
    let expected = ExpectedLaunch::new(
        ExpectedState::AbsentCanonical,
        ServerId::Ambient,
        "1".to_owned(),
        "2".to_owned(),
        "/dev/ttys004".to_owned(),
        "3".to_owned(),
        4,
    );
    let argv = expected_launch_argv(&preamble, &user, &expected);
    assert!(!argv.iter().any(|word| word == "--test-pre-lock-marker"));

    #[cfg(debug_assertions)]
    {
        let mut opted_in = expected;
        opted_in.enable_test_pre_lock_marker();
        let argv = expected_launch_argv(&preamble, &user, &opted_in);
        assert_eq!(
            argv.iter()
                .filter(|word| word.as_str() == "--test-pre-lock-marker")
                .count(),
            1
        );
        let marker_at = argv
            .iter()
            .position(|word| word == "--test-pre-lock-marker");
        let separator = argv.iter().position(|word| word == "--");
        assert!(marker_at < separator, "{argv:?}");
    }
}

#[test]
fn the_lead_pair_sets_equal_width_before_selecting_the_main_vertical_layout() {
    let panes = ["%0".to_owned(), "%1".to_owned()];
    assert_eq!(
        pair_policy_words("lead-pair", &panes[0]),
        vec![
            vec!["set-window-option", "-t", "%0", "main-pane-width", "50%"],
            vec![
                "if-shell",
                "-F",
                "-t",
                "%0",
                "#{==:#{window_zoomed_flag},0}",
                "select-layout -t %0 main-vertical"
            ],
            vec![
                "set-hook",
                "-w",
                "-t",
                "%0",
                "window-resized",
                "if-shell -F -t %0 '#{&&:#{==:#{window_zoomed_flag},0},#{==:#{P:#{@ae_reader_src}},}}' 'select-layout -t %0 main-vertical'"
            ],
        ]
    );
    assert!(pair_policy_words("vertical", "%0").is_empty());

    let panes = [
        "%0".to_owned(),
        "%1".to_owned(),
        "%2".to_owned(),
        "%3".to_owned(),
    ];
    assert_eq!(
        layout_words("lead-pair", &panes, 3),
        vec![vec!["select-layout", "-t", "%2", "even-vertical"]],
        "extra standing workers keep their separate stacked window"
    );
}

#[test]
fn every_non_pair_layout_keeps_its_existing_layout_program() {
    let panes = ["%0".to_owned(), "%1".to_owned(), "%2".to_owned()];
    assert_eq!(
        layout_words("lead-solo", &panes, 2),
        vec![vec!["select-layout", "-t", "%1", "even-vertical"]]
    );
    assert_eq!(
        layout_words("vertical", &panes, 2),
        vec![vec!["select-layout", "-t", "=named:", "even-horizontal"]]
    );
    assert_eq!(
        layout_words("horizontal", &panes, 2),
        vec![vec!["select-layout", "-t", "=named:", "even-vertical"]]
    );
}

#[test]
fn automatic_upgrade_hooks_follow_reattach_validation_and_fresh_launch_completion() {
    let source = include_str!("../session_launch.rs");
    let reattach = source
        .split_once("// ---- a session that is already running is reattached")
        .and_then(|(_, tail)| tail.split_once("if seeds_orchestrator_config"))
        .map(|(body, _)| body)
        .expect("running-session branch");
    let validated = reattach
        .find("observe_agents")
        .expect("ae-session validation");
    let released = reattach
        .find("drop(lifecycle.take())")
        .expect("lifecycle guard release");
    let scheduled = reattach
        .find("autoupgrade::schedule")
        .expect("reattach scheduling edge");
    let attach = reattach.find("if !env.attach").expect("attach decision");
    assert!(
        validated < released && released < scheduled && scheduled < attach,
        "{reattach}"
    );

    let completed = source
        .split_once("// ---- the Telegram bridge ----")
        .and_then(|(_, tail)| {
            tail.split_once(
                "// ---------------------------------------------------------------------------",
            )
        })
        .map(|(body, _)| body)
        .expect("completed launch tail");
    let selected = completed
        .find("Op::SelectPane")
        .expect("final pane selection");
    let scheduled = completed
        .find("autoupgrade::schedule")
        .expect("fresh-launch scheduling edge");
    let attach = completed.find("if !env.attach").expect("attach decision");
    assert!(selected < scheduled && scheduled < attach, "{completed}");
}

#[test]
fn settings_launch_preserves_floor_then_locks_before_authoritative_proof_and_effects() {
    let source = include_str!("../session_launch.rs");
    let launch = source
        .split_once("let proposed_server = env.server();")
        .map(|(_, tail)| tail)
        .expect("launch policy");
    let floor = launch.find("floor_refusal").expect("tmux floor");
    let locked = launch.find("crate::store::lock").expect("lifecycle lock");
    // The binding and its first operand, whatever line break rustfmt puts
    // between them: the read under the lock that the expected action decides.
    let meta_present = launch
        .find("let meta_present =")
        .filter(|at| {
            launch[at + "let meta_present =".len()..]
                .trim_start()
                .starts_with("if expected_launch.is_some()")
        })
        .expect("locked state read");
    let clicker = launch.find("expected.check_action").expect("action proof");
    let census = launch
        .find("prove_absent_start")
        .expect("raw-role census and absence proof");
    let seeded = launch
        .find("seed_default_config")
        .expect("first-run config seed");
    let built = launch.find("build(").expect("existing launch build");
    assert!(
        floor < locked
            && locked < meta_present
            && meta_present < clicker
            && clicker < census
            && census < seeded
            && seeded < built,
        "{launch}"
    );

    let resume_guard = launch
        .find("expected_launch.is_some_and(ExpectedLaunch::is_resume)")
        .expect("expected Resume lock condition");
    let stopped = launch
        .find("prove_stopped_role")
        .expect("locked stopped-role proof");
    assert!(resume_guard < locked && locked < stopped, "{launch}");
}
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-launch-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn watchdog_registration_is_cleared_before_a_fast_replacement_publishes() {
    let dir = scratch("watchdog-replacement-order");
    std::fs::write(crate::watchdog_glue::pidfile(&dir), "41\n").unwrap();

    let (replaced, _) = replace_watchdog_registration(&dir, || {
        assert_eq!(
            crate::watchdog_glue::read_pid(&dir),
            None,
            "the old registration must be gone before respawn can publish"
        );
        std::fs::write(crate::watchdog_glue::pidfile(&dir), "42\n").unwrap();
        (true, String::new())
    });

    assert!(replaced);
    assert_eq!(
        crate::watchdog_glue::read_pid(&dir),
        Some(42),
        "the replacement registration survives"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_public_launch_parser_owns_directory_and_attach_flags() {
    let args = ["named", "--dir", "/repo", "--no-attach", "--worktree"].map(str::to_owned);
    let plan = parse_plan(&args).expect("public launch flags");
    assert_eq!(plan.name.as_deref(), Some("named"));
    assert_eq!(plan.dir.as_deref(), Some("/repo"));
    assert_eq!(plan.attach, Some(false));
    assert_eq!(plan.mode, Some(super::Mode::Git));

    assert!(parse_plan(&["--dir".to_owned()]).is_err());
    assert!(parse_plan(&["--dir=".to_owned()]).is_err());
    assert!(parse_plan(&["--attach".to_owned()]).is_err());
    assert!(
        parse_plan(&[
            "--dir=/one".to_owned(),
            "--dir".to_owned(),
            "/two".to_owned(),
        ])
        .is_err()
    );
}

/// Whether `check` refused, and what it wrote to stderr.
fn refused_with(
    check: impl FnOnce(&mut Vec<u8>) -> crate::error::Result<std::ops::ControlFlow<u8, ()>>,
) -> (Option<u8>, String) {
    let mut err = Vec::new();
    let flow = check(&mut err).expect("the check writes to a buffer");
    let code = match flow {
        std::ops::ControlFlow::Break(code) => Some(code),
        std::ops::ControlFlow::Continue(()) => None,
    };
    (code, String::from_utf8_lossy(&err).into_owned())
}

#[test]
fn only_a_stopped_near_miss_session_without_a_sign_of_intent_is_refused() {
    // (name, deliberate, record present, running) -> refused?
    for (name, deliberate, present, running, refused) in [
        ("sessions", false, true, false, true),
        ("lists", false, true, false, true),
        ("sessions", true, true, false, false),
        ("sessions", false, true, true, false),
        ("sessions", false, false, false, false),
        ("plain", false, true, false, false),
    ] {
        let (code, text) =
            refused_with(|err| check_stopped_near_miss(name, deliberate, present, running, err));
        let label = format!("{name} deliberate={deliberate} present={present} running={running}");
        if refused {
            assert_eq!(code, Some(EXIT_USAGE), "{label}");
            assert_eq!(
                text,
                format!(
                    "{}\n",
                    crate::entry::near_miss_resume_refusal(name).expect("a near-miss word")
                ),
                "{label}"
            );
        } else {
            assert_eq!((code, text.as_str()), (None, ""), "{label}");
        }
    }
}

#[test]
fn resume_refuses_only_where_there_is_no_record_and_no_proof_of_one() {
    // (resume, carries own proof, record present) -> refused?
    for (resume, own_proof, present, refused) in [
        (true, false, false, true),
        (true, false, true, false),
        (true, true, false, false),
        (false, false, false, false),
    ] {
        let (code, text) =
            refused_with(|err| check_resume_target("fresh", resume, own_proof, present, err));
        let label = format!("resume={resume} own_proof={own_proof} present={present}");
        if refused {
            assert_eq!(code, Some(EXIT_USAGE), "{label}");
            assert_eq!(
                text,
                "Error: --resume found no session named 'fresh' to resume. Nothing was created.\n",
                "{label}"
            );
        } else {
            assert_eq!((code, text.as_str()), (None, ""), "{label}");
        }
    }
}

#[test]
fn a_launch_carrying_an_expectation_is_not_asked_for_a_sign_of_intent() {
    let expected = ExpectedLaunch::new(
        ExpectedState::AbsentCanonical,
        ServerId::Ambient,
        "1".to_owned(),
        "2".to_owned(),
        "/dev/ttys004".to_owned(),
        "3".to_owned(),
        4,
    );
    assert!(carries_own_proof(Some(&expected), None));
    assert!(!carries_own_proof(None, None));

    // `--resume` for a name with no record: refused for a bare launch, passed
    // for one that carries an expectation (nothing is read from the record).
    let dir = scratch("own-proof");
    for (proof, refused) in [(None, true), (Some(&expected), false)] {
        let mut err = Vec::new();
        let flow = check_launch_kind("fresh", &dir, proof, None, true, false, false, &mut err)
            .expect("the check writes to a buffer");
        let text = String::from_utf8_lossy(&err).into_owned();
        if refused {
            assert_eq!(flow, std::ops::ControlFlow::Break(EXIT_USAGE), "{text}");
            assert_eq!(
                text,
                "Error: --resume found no session named 'fresh' to resume. Nothing was created.\n"
            );
        } else {
            assert_eq!(flow, std::ops::ControlFlow::Continue(false), "{text}");
            assert_eq!(text, "");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The hidden launch child's argv for session `session`: the state root, the
/// cwd, the global config and the private server of a test, with the debug
/// marker.
#[cfg(debug_assertions)]
fn launch_words(
    session: &str,
    home: &std::path::Path,
    cwd: &std::path::Path,
    config: &std::path::Path,
    socket: &std::path::Path,
) -> Vec<String> {
    let path = |path: &std::path::Path| path.to_str().expect("a utf-8 scratch path").to_owned();
    vec![
        "--home".to_owned(),
        path(home),
        "--cwd".to_owned(),
        path(cwd),
        "--global".to_owned(),
        path(config),
        "--server-kind".to_owned(),
        "socket".to_owned(),
        "--server".to_owned(),
        path(socket),
        "--no-attach".to_owned(),
        "--test-pre-lock-marker".to_owned(),
        "--".to_owned(),
        session.to_owned(),
    ]
}

/// The private global config of the lock-race tests: one idle profile that
/// names no real agent, and nothing that opens a window.
#[cfg(debug_assertions)]
fn lock_race_config(root: &std::path::Path) -> PathBuf {
    let config = root.join("config");
    std::fs::write(
        &config,
        "[profiles]\nidle = \"/nonexistent/ae-agent\"\n[roster]\nlead = idle\n\
         [workspace]\nmain = lead\nworkers =\nwatchdog = false\nchat = off\ntheme = off\n",
    )
    .unwrap();
    config
}

/// A lock-race test's scratch root and the private tmux server its launch may
/// start at `<root>/sock`. A launch that wrongly starts a session starts a real
/// server there, and the lane's sweep does not know this root, so the drop ends
/// that exact server and removes the root on every exit, a panic included.
#[cfg(debug_assertions)]
struct Fixture {
    root: PathBuf,
    server: ServerId,
}

#[cfg(debug_assertions)]
impl Fixture {
    fn new(tag: &str) -> Self {
        let root = scratch(tag);
        let server = ServerId::Selected(crate::meta::Selector::Socket(root.join("sock")));
        Self { root, server }
    }
}

#[cfg(debug_assertions)]
impl Drop for Fixture {
    fn drop(&mut self) {
        use crate::transport::fixture_tmux;
        let socket = self.root.join("sock");
        let _ = fixture_tmux::end(&self.server);
        let mut last = fixture_tmux::probe(&self.server);
        for _ in 0..20 {
            // A server that never started left no socket, and one that ended
            // took its socket with it.
            if last.proven() || !socket.exists() {
                let _ = std::fs::remove_dir_all(&self.root);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            last = fixture_tmux::probe(&self.server);
        }
        assert!(
            std::thread::panicking(),
            "fixture socket cleanup unproven: {last:?}"
        );
    }
}

/// What one launch did, and where it did it.
#[cfg(debug_assertions)]
struct LaunchRun {
    fixture: Fixture,
    code: u8,
    out: String,
    err: String,
}

#[cfg(debug_assertions)]
impl LaunchRun {
    fn root(&self) -> &std::path::Path {
        &self.fixture.root
    }

    fn home(&self) -> PathBuf {
        self.root().join("home")
    }

    fn socket(&self) -> PathBuf {
        self.root().join("sock")
    }
}

/// A picker resume of the stopped session `s`, whose record the launch finds at
/// its preflight. The test holds the session's real lifecycle lock, so the
/// launch passes that preflight, publishes the debug marker and waits for the
/// lock. Then `change` runs once, with the sessions root and the scratch root,
/// and the lock is let go.
#[cfg(debug_assertions)]
fn picker_resume_racing_the_lock(
    tag: &str,
    change: impl FnOnce(&std::path::Path, &std::path::Path) -> std::io::Result<()> + Send + 'static,
) -> LaunchRun {
    let fixture = Fixture::new(tag);
    let root = fixture.root.clone();
    let home = root.join("home");
    let sessions = home.join("sessions");
    let dir = sessions.join("s");
    let socket = root.join("sock");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("meta"),
        format!(
            "schema=2\nmeta_version=2\nmode=local\nsession=s\n\
             session_id=11111111-1111-4111-8111-111111111111\ncreated=1\nstarted=1\n\
             work_dir={w}\norigin={w}\ntmux_server_kind=socket\ntmux_server={s}\n\
             seat.main=lead\nprofile.main=idle\nagent_bin.main=/nonexistent/ae-agent\n",
            w = root.display(),
            s = socket.display(),
        ),
    )
    .unwrap();
    let config = lock_race_config(&root);

    let held = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(sessions.join(".lifecycle.s.lock"))
        .expect("the lifecycle lock opens");
    held.try_lock().expect("the test holds the lifecycle lock");
    let marker = home.join(super::TEST_PRE_LOCK_MARKER);
    let racer = {
        let (sessions, root) = (sessions.clone(), root.clone());
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !marker.is_file() {
                if std::time::Instant::now() > deadline {
                    return Err("the launch never reached its pre-lock marker".to_owned());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            change(&sessions, &root).map_err(|why| why.to_string())?;
            drop(held);
            Ok(())
        })
    };

    let words = launch_words("s", &home, &root, &config, &socket);
    let expected = ExpectedLaunch::unattended(
        ExpectedState::StoppedSession,
        crate::time::Timestamp::now().epoch() + 60,
    );
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = super::run_with_expected(
        &words,
        Some(&expected),
        None,
        None,
        (&super::LaunchCaller::Outside, super::LaunchRoute::Picker),
        &mut out,
        &mut err,
    )
    .expect("the launch writes to buffers");
    let barrier = racer.join().expect("the racer ends");
    let (out, err) = (
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    );
    assert_eq!(barrier, Ok(()), "{err}");
    LaunchRun {
        fixture,
        code,
        out,
        err,
    }
}

/// The suffix of the refusal a resume whose record is gone under the lock
/// prints: the words before it explain, these words promise what it did.
#[cfg(debug_assertions)]
const RESUME_REFUSED_SUFFIX: &str = "Nothing was resumed.";

/// A refused resume: the failed exit and the promise that nothing was resumed.
#[cfg(debug_assertions)]
fn assert_resume_refused(run: &LaunchRun) {
    let LaunchRun { code, out, err, .. } = run;
    assert_eq!(*code, super::EXIT_FAILED, "{out}{err}");
    assert!(
        err.trim_end().ends_with(RESUME_REFUSED_SUFFIX),
        "the refusal does not end by saying nothing was resumed: {err}"
    );
}

/// What a refused resume must not have made: a worktree for the session, a tmux
/// server, or any claim of being resumed.
#[cfg(debug_assertions)]
fn assert_nothing_was_created(run: &LaunchRun) {
    let LaunchRun { out, err, .. } = run;
    assert!(
        !out.contains("Resuming session"),
        "a record the lock does not hold is never resumed: {out}"
    );
    assert!(
        !run.home().join("worktrees").join("s").exists(),
        "the refusal made a worktree: {err}"
    );
    assert!(
        !run.socket().exists(),
        "the refusal started a tmux server: {err}"
    );
}

/// DEFEATING MUTATION: turn the first `||` of `meta_present` in `launch`
/// (`expected_launch.is_some() || audit.is_some() || …`) into `&&` and this
/// test goes RED. An expected launch with no audit and no autostart then
/// decides by the preflight value instead of by what its lifecycle lock holds,
/// and a record that vanished while it waited is read as present.
///
/// The oracle is the comment above that line, not a recorded output: "An
/// expected or audited launch decides by what the lock holds". The picker's
/// resume proves no record of its own under the lock (`StoppedSession`), so
/// the locked read is its only judge. The record goes while the launch waits,
/// its directory stays and a worktree directory of the name is left behind:
/// judged by the lock, the record is gone and the resume is refused; judged by
/// the stale preflight, it is a resume that cannot read the record it expects.
#[cfg(debug_assertions)]
#[test]
fn an_expected_launch_decides_by_the_record_its_lock_holds_not_the_preflight_one() {
    let run = picker_resume_racing_the_lock("expected-lock-record", |sessions, _| {
        std::fs::remove_file(sessions.join("s").join("meta"))?;
        let home = sessions.parent().expect("the sessions root has a home");
        std::fs::create_dir_all(home.join("worktrees").join("s"))
    });
    assert!(
        !run.err.contains("could not read metadata"),
        "the launch read the record the lock no longer holds: {}",
        run.err
    );
    assert_resume_refused(&run);
    assert!(
        !run.out.contains("Resuming session"),
        "a record the lock does not hold is never resumed: {}",
        run.out
    );
    assert!(
        !run.socket().exists(),
        "the refusal started a tmux server: {}",
        run.err
    );
    assert!(
        !run.home().join("sessions").join("s").join("meta").exists(),
        "the refusal wrote a record"
    );
}

/// The picker row promised a resume. A session that is gone under the lock
/// (ended while the click waited) is refused, and no fresh session of that
/// name appears, with no state directory, no worktree and no server.
#[cfg(debug_assertions)]
#[test]
fn a_picker_resume_of_a_session_that_vanished_under_the_lock_refuses_and_creates_nothing() {
    let run = picker_resume_racing_the_lock("resume-vanished", |sessions, _| {
        std::fs::remove_dir_all(sessions.join("s"))
    });
    assert_resume_refused(&run);
    assert_nothing_was_created(&run);
    assert!(
        !run.home().join("sessions").join("s").exists(),
        "the refusal made a state directory"
    );
}

/// The record a live rename of `s` to `t` publishes under the new name.
#[cfg(debug_assertions)]
const RENAMED_RECORD: &str = "schema=2\nmeta_version=2\nmode=local\nsession=t\n\
                              session_id=11111111-1111-4111-8111-111111111111\n";

/// A live rename between the preflight and the lock leaves its helper alias at
/// the old name. That name holds no record: the resume is refused, and the
/// refusal leaves the alias, which belongs to the rename, where it is, and the
/// renamed session's record as the rename published it.
#[cfg(debug_assertions)]
#[test]
fn a_picker_resume_whose_name_became_a_rename_alias_under_the_lock_refuses_and_keeps_it() {
    let run = picker_resume_racing_the_lock("resume-aliased", |sessions, _| {
        std::fs::rename(sessions.join("s"), sessions.join("t"))?;
        std::fs::write(sessions.join("t").join("meta"), RENAMED_RECORD)?;
        std::os::unix::fs::symlink("t", sessions.join("s"))
    });
    assert_resume_refused(&run);
    assert_nothing_was_created(&run);
    let sessions = run.home().join("sessions");
    assert_eq!(
        std::fs::read_link(sessions.join("s")).ok(),
        Some(PathBuf::from("t")),
        "the refusal reclaimed the rename's alias"
    );
    assert_eq!(
        std::fs::read_to_string(sessions.join("t").join("meta")).ok(),
        Some(RENAMED_RECORD.to_owned()),
        "the refusal changed the renamed session's record"
    );
}

/// A link that is no recognized rename alias keeps the launch's own refusal,
/// whatever sits behind it: the resume never reaches a server through it.
#[cfg(debug_assertions)]
#[test]
fn a_picker_resume_whose_name_became_a_foreign_link_under_the_lock_still_refuses_it() {
    let run = picker_resume_racing_the_lock("resume-foreign-link", |sessions, root| {
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere)?;
        std::fs::write(elsewhere.join("meta"), "session=s\n")?;
        std::fs::remove_dir_all(sessions.join("s"))?;
        std::os::unix::fs::symlink(&elsewhere, sessions.join("s"))
    });
    assert_eq!(run.code, super::EXIT_FAILED, "{}{}", run.out, run.err);
    assert!(
        run.err
            .contains("is a symlink; refusing to launch through it"),
        "{}",
        run.err
    );
    assert_nothing_was_created(&run);
    let kept: Vec<_> = std::fs::read_dir(run.root().join("elsewhere"))
        .expect("the link target stays")
        .map(|entry| entry.expect("an entry").file_name())
        .collect();
    assert_eq!(kept, ["meta"], "the launch wrote through the link");
}

/// The alias reclaim is skipped for a resume only. A settings Start of the
/// canonical name still reclaims a recognized alias before its own proof, which
/// here refuses (a sibling directory with no record makes the role census
/// fail): the alias is gone when it does.
#[cfg(debug_assertions)]
#[test]
fn an_expected_start_still_reclaims_a_rename_alias_before_its_proof() {
    let fixture = Fixture::new("start-reclaims-alias");
    let root = fixture.root.clone();
    let home = root.join("home");
    let sessions = home.join("sessions");
    let socket = root.join("sock");
    std::fs::create_dir_all(sessions.join("x")).unwrap();
    let canonical = sessions.join(crate::orchestrator::ORCHESTRATOR_SESSION);
    std::os::unix::fs::symlink("x", &canonical).unwrap();
    let config = lock_race_config(&root);

    let words = launch_words(
        crate::orchestrator::ORCHESTRATOR_SESSION,
        &home,
        &root,
        &config,
        &socket,
    );
    let expected = ExpectedLaunch::unattended(
        ExpectedState::AbsentCanonical,
        crate::time::Timestamp::now().epoch() + 60,
    );
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = super::run_with_expected(
        &words,
        Some(&expected),
        None,
        None,
        (&super::LaunchCaller::Outside, super::LaunchRoute::Picker),
        &mut out,
        &mut err,
    )
    .expect("the launch writes to buffers");
    let (out, err) = (
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    );
    assert_eq!(code, super::EXIT_FAILED, "{out}{err}");
    assert!(
        err.contains("Nothing was started."),
        "the Start's own proof refused: {err}"
    );
    assert!(
        std::fs::read_link(&canonical).is_err() && !canonical.exists(),
        "the Start reclaimed the alias before its proof: {err}"
    );
    assert!(sessions.join("x").is_dir(), "the sibling stays");
}

#[test]
fn a_compact_child_creates_so_a_near_miss_name_is_refused_before_a_write() {
    let home = scratch("relaunch-near-miss");
    let origin = home.to_string_lossy().into_owned();
    let child = Relaunch {
        home: &home,
        name: "sessions",
        mode: "local",
        origin: &origin,
        config: "",
        uuid: "11111111-1111-4111-8111-111111111111",
        proof: "none",
        roster: "main=lead workers=-",
        server_kind: "socket",
        server_value: "/nonexistent/ae-test-server",
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = relaunch(&child, &mut out, &mut err).expect("the launch writes to buffers");
    assert_eq!(code, EXIT_USAGE);
    assert_eq!(
        String::from_utf8_lossy(&err),
        format!(
            "{}\n",
            crate::entry::near_miss_refusal("sessions").expect("a near-miss word")
        )
    );
    assert!(out.is_empty());
    assert!(
        !home.join("sessions").exists(),
        "the refusal wrote no session state"
    );
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn resume_is_a_public_launch_flag_the_seat_grammar_refuses() {
    let words = |argv: &[&str]| {
        argv.iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>()
    };
    let plan = parse_plan(&words(&["demo", "--resume", "--no-attach"])).expect("parses");
    assert!(plan.resume && plan.attach == Some(false));
    assert!(!parse_plan(&words(&["demo"])).expect("plain").resume);
    let path = parse_plan(&words(&["demo", "--dir", "--resume"])).expect("a path");
    assert_eq!(path.dir.as_deref(), Some("--resume"));
    assert!(!path.resume, "a --dir value is never the flag");
    assert!(parse_plan_with_attach(&words(&["--resume"]), true).is_err());
    assert!(super::PUBLIC_USAGE.contains("[--resume]") && super::USAGE.contains("[--resume]"));
}

#[test]
fn tmux_is_a_public_launch_flag_the_seat_grammar_refuses() {
    let words = |argv: &[&str]| {
        argv.iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>()
    };
    let plan = parse_plan(&words(&["demo", "--tmux", "--tmux"])).expect("--tmux parses");
    assert!(plan.tmux && plan.attach.is_none());
    let both = parse_plan(&words(&["--no-attach", "demo", "--tmux"])).expect("both parse");
    assert!(both.tmux && both.attach == Some(false));
    let path = parse_plan(&words(&["demo", "--dir", "--tmux"])).expect("a path");
    assert_eq!(path.dir.as_deref(), Some("--tmux"));
    assert!(!path.tmux, "a --dir value is never the flag");
    assert!(parse_plan_with_attach(&words(&["--tmux"]), true).is_err());
    assert!(super::PUBLIC_USAGE.contains("[--tmux]") && super::USAGE.contains("[--tmux]"));
}

#[test]
fn the_public_launch_parser_owns_general_and_sugar_seat_profiles() {
    let plan = parse_plan(
        &[
            "named",
            "--seat",
            "builder=terrax",
            "--lead",
            "solx",
            "--colead=astrax",
        ]
        .map(str::to_owned),
    )
    .expect("seat profile flags");
    assert_eq!(
        plan.seat_profiles,
        [
            ("builder".to_owned(), "terrax".to_owned()),
            ("lead".to_owned(), "solx".to_owned()),
            ("colead".to_owned(), "astrax".to_owned()),
        ]
    );

    for args in [
        vec!["--seat", "lead=one", "--seat=lead=two"],
        vec!["--lead", "one", "--seat", "lead=two"],
        vec!["--seat"],
        vec!["--seat", "lead"],
        vec!["--seat", "=profile"],
        vec!["--seat", "lead="],
        vec!["--colead"],
    ] {
        assert!(parse_plan(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
    }
}

#[test]
fn the_public_launch_parser_owns_the_solo_roster_override() {
    let plan = parse_plan(&["named", "--solo", "--lead", "solx"].map(str::to_owned))
        .expect("a solo launch with a lead profile override");
    assert_eq!(plan.workers.as_deref(), Some(""));
    assert_eq!(plan.seat_profiles, [("lead".to_owned(), "solx".to_owned())]);

    for args in [
        ["named", "--solo", "--colead", "astrax"],
        ["named", "--colead", "astrax", "--solo"],
    ] {
        assert_eq!(
            parse_plan(&args.map(str::to_owned)),
            Err("Error: '--solo' starts the lead alone; drop --colead/--seat <worker>.".to_owned())
        );
    }
}

#[test]
fn a_second_positional_is_a_usage_error_naming_both_words() {
    let err = parse_plan(&["aedevx".to_owned(), "msg".to_owned()]).expect_err("two names refuse");
    assert!(
        err.contains("'aedevx'") && err.contains("'msg'"),
        "both words named: {err}"
    );
    assert!(
        !err.contains("session helper"),
        "no helper named, no hint: {err}"
    );
}

#[test]
fn a_helper_shaped_second_positional_names_the_helper_call() {
    let err = parse_plan(&["aedevx", "send", "colead", "msg"].map(str::to_owned))
        .expect_err("a mistyped helper call refuses");
    assert!(
        err.contains("'aedevx'") && err.contains("'send'"),
        "first and second named: {err}"
    );
    assert!(
        err.contains("'send' is a session helper")
            && err.contains("<state-root>/sessions/<session>/send"),
        "the helper hint: {err}"
    );
    let err = parse_plan(&["@", "send", "x", "y"].map(str::to_owned))
        .expect_err("no name survives '@ send x y'");
    assert!(
        err.contains("'@'") && err.contains("'send'"),
        "both words named: {err}"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one table pinning every accepted launch spelling to its plan"
)]
fn every_existing_launch_spelling_still_parses_to_the_same_plan() {
    let name = |session: &str| Plan {
        name: Some(session.to_owned()),
        ..Plan::default()
    };
    let uuid = "0199c0de-1234-4890-abcd-ef0123456789";
    let cases: &[(&[&str], Plan)] = &[
        (&["demo"], name("demo")),
        (
            &["demo", "use", "colead"],
            Plan {
                main: Some("colead".to_owned()),
                ..name("demo")
            },
        ),
        (
            &["--dir", "/repo", "demo"],
            Plan {
                dir: Some("/repo".to_owned()),
                ..name("demo")
            },
        ),
        (
            &["--dir=/repo", "demo"],
            Plan {
                dir: Some("/repo".to_owned()),
                ..name("demo")
            },
        ),
        (
            &["--from", uuid, "demo"],
            Plan {
                from: Some(uuid.to_owned()),
                ..name("demo")
            },
        ),
        (
            &["--from=0199c0de-1234-4890-abcd-ef0123456789", "demo"],
            Plan {
                from: Some(uuid.to_owned()),
                ..name("demo")
            },
        ),
        (
            &["--lead", "solx", "demo"],
            Plan {
                seat_profiles: vec![("lead".to_owned(), "solx".to_owned())],
                ..name("demo")
            },
        ),
        (
            &["--lead=solx", "demo"],
            Plan {
                seat_profiles: vec![("lead".to_owned(), "solx".to_owned())],
                ..name("demo")
            },
        ),
        (
            &["--colead", "astrax", "demo"],
            Plan {
                seat_profiles: vec![("colead".to_owned(), "astrax".to_owned())],
                ..name("demo")
            },
        ),
        (
            &["--colead=astrax", "demo"],
            Plan {
                seat_profiles: vec![("colead".to_owned(), "astrax".to_owned())],
                ..name("demo")
            },
        ),
        (
            &["--seat", "builder=terrax", "demo"],
            Plan {
                seat_profiles: vec![("builder".to_owned(), "terrax".to_owned())],
                ..name("demo")
            },
        ),
        (
            &["--seat=builder=terrax", "demo"],
            Plan {
                seat_profiles: vec![("builder".to_owned(), "terrax".to_owned())],
                ..name("demo")
            },
        ),
        (
            &["demo", "--solo"],
            Plan {
                workers: Some(String::new()),
                ..name("demo")
            },
        ),
        (
            &["demo", "--no-attach"],
            Plan {
                attach: Some(false),
                ..name("demo")
            },
        ),
        (
            &["--local", "demo"],
            Plan {
                mode: Some(Mode::Local),
                ..name("demo")
            },
        ),
        (
            &["--copy", "demo"],
            Plan {
                mode: Some(Mode::Full),
                ..name("demo")
            },
        ),
        (
            &["--worktree", "demo"],
            Plan {
                mode: Some(Mode::Git),
                ..name("demo")
            },
        ),
    ];
    for (argv, expected) in cases {
        let words: Vec<String> = argv.iter().map(|word| (*word).to_owned()).collect();
        assert_eq!(parse_plan(&words), Ok(expected.clone()), "{argv:?}");
    }
    // The attach path's own spellings: `--attach` parses, and a seat tail
    // with a name still parses (the orchestrator rejects the name itself).
    assert_eq!(
        parse_plan_with_attach(&["--attach".to_owned()], true),
        Ok(Plan {
            attach: Some(true),
            ..Plan::default()
        })
    );
    assert_eq!(
        parse_plan_with_attach(&["demo".to_owned(), "--attach".to_owned()], true)
            .expect("a name with --attach parses")
            .name
            .as_deref(),
        Some("demo")
    );
}

#[test]
fn switching_to_the_session_already_owning_the_caller_is_in_place() {
    assert_eq!(
        attach_action(true, true, Some("inside"), "inside"),
        AttachAction::InPlace
    );
    assert_eq!(
        attach_action(true, true, Some("inside"), "other"),
        AttachAction::Focus(crate::tmux::FocusVerb::SwitchClient)
    );
    assert_eq!(
        attach_action(false, false, Some("inside"), "inside"),
        AttachAction::Focus(crate::tmux::FocusVerb::AttachSession)
    );
    assert_eq!(
        attach_action(true, false, Some("inside"), "inside"),
        AttachAction::Hint
    );
}

#[test]
fn fleet_attach_hints_keep_the_selected_server_spelling() {
    use crate::meta::Selector;

    assert_eq!(
        server_attach_hint(&ServerId::Selected(Selector::Name("ae".to_owned()))),
        "tmux -L ae attach"
    );
    assert_eq!(
        server_attach_hint(&ServerId::Selected(Selector::Socket("/tmp/ae.sock".into()))),
        "tmux -S /tmp/ae.sock attach"
    );
}

/// EVERY codex resume needs the turn pasted, including the one whose id is
/// still `pending` and which therefore launches by the fresh fallback.
#[test]
fn every_codex_resume_gets_the_turn_pasted_and_no_fresh_start_does() {
    assert!(launch_turn_is_pasted(ToolKind::Codex, true));
    assert!(
        !launch_turn_is_pasted(ToolKind::Codex, false),
        "a create bakes the turn into argv; pasting it too would double it"
    );
    for tool in [
        ToolKind::Claude,
        ToolKind::Gemini,
        ToolKind::Agy,
        ToolKind::Grok,
        ToolKind::OpenCode,
        ToolKind::Unknown,
    ] {
        assert!(!launch_turn_is_pasted(tool, true), "{tool:?}");
    }
}

/// A seat KEEPS its launch id across a resume; only a seat without one is
/// given a fresh one. Capture capability never changes that seat identity.
#[test]
fn a_resumed_seat_keeps_the_launch_id_that_guards_its_identity() {
    assert_eq!(
        launch_token(ToolKind::Codex, Some("tok-1".to_owned())),
        "tok-1",
        "dropping it leaves the capture matching by working directory alone"
    );
    let minted = launch_token(ToolKind::Codex, None);
    assert!(!minted.is_empty() && minted != "tok-1");
    assert!(
        !launch_token(ToolKind::Codex, Some(String::new())).is_empty(),
        "an empty row is no token at all: mint one"
    );
    for tool in [ToolKind::Claude, ToolKind::Grok, ToolKind::Unknown] {
        assert_eq!(
            launch_token(tool, Some("tok-1".to_owned())),
            "tok-1",
            "{tool:?}"
        );
        assert!(
            !launch_token(tool, None).is_empty(),
            "every seat gets a meta guard: {tool:?}"
        );
    }
}

/// An empty `harness_session` row resumes like a `pending` one: only a
/// stored id survives the filter, so a hand-emptied row re-captures
/// instead of naming a conversation that does not exist.
#[test]
fn an_empty_harness_session_row_resumes_like_a_pending_one() {
    let dir = scratch("empty-harness-row");
    for (row, want) in [
        ("harness_session.main=\n", None),
        ("harness_session.main=pending\n", None),
        ("schema=2\n", None),
        (
            "harness_session.main=0199c0de-1234-4890-abcd-ef0123456789\n",
            Some("0199c0de-1234-4890-abcd-ef0123456789".to_owned()),
        ),
    ] {
        std::fs::write(dir.join("meta"), row).unwrap();
        assert_eq!(
            super::stored_session_id(&dir, "main", true),
            want,
            "{row:?}"
        );
    }
    std::fs::write(dir.join("meta"), "harness_session.main=sid\n").unwrap();
    assert_eq!(
        super::stored_session_id(&dir, "main", false),
        None,
        "a fresh launch reads no row"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A resume caps `events.jsonl` at its NEWEST lines, and the cut falls
/// exactly at the retention boundary.
#[test]
fn a_trim_keeps_the_newest_events_and_cuts_at_the_boundary() {
    let dir = scratch("trim");
    let seeded = EVENTS_KEEP + 200;
    let mut text = String::new();
    for index in 1..=seeded {
        let _ = writeln!(text, "{{\"seq\":{index}}}");
    }
    std::fs::write(dir.join("events.jsonl"), &text).unwrap();

    trim_events(&dir);

    let kept = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
    let lines: Vec<&str> = kept.lines().collect();
    assert_eq!(lines.len(), EVENTS_KEEP);
    assert_eq!(lines.first().copied(), Some("{\"seq\":201}"));
    assert_eq!(
        lines.last().copied(),
        Some(format!("{{\"seq\":{seeded}}}").as_str())
    );
    assert!(!kept.contains("{\"seq\":200}"), "the boundary line is cut");
    // The temp the rename came from does not survive the operation.
    let residue: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&format!("{}.trim.", crate::store::EVENTS)))
        .collect();
    assert!(residue.is_empty(), "no trim temp is left: {residue:?}");

    // A log already under the cap is left exactly as it was.
    std::fs::write(dir.join("events.jsonl"), "{\"seq\":1}\n").unwrap();
    trim_events(&dir);
    assert_eq!(
        std::fs::read_to_string(dir.join("events.jsonl")).unwrap(),
        "{\"seq\":1}\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// DEFEATING MUTATION: delete the observed-model enumeration from
/// `meta_document` and this test goes RED. The document is the WHOLE meta,
/// so a pair not enumerated here is deleted by the first resume — before
/// any resume could honor it.
#[test]
fn a_resume_document_carries_the_observed_model_pair_forward() {
    let root = scratch("observed-model-document");
    let home = root.join("home");
    let dir = home.join("sessions").join("s");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("meta"),
        "schema=2\nmeta_version=2\nmode=local\nsession=s\nsession_id=SID\ncreated=1\n\
             started=1\nwork_dir=/w\norigin=/o\nseat.main=lead\nprofile.main=fable5\n\
             agent_bin.main=claude\nharness_session.main=sid\nlaunch_id.main=L1\n\
             observed_model.main=Opus 5 (1M context)\nobserved_model_pin.main=fable\n",
    )
    .unwrap();
    let env = super::Env {
        home: home.clone(),
        cwd: PathBuf::from("/w"),
        global: None,
        local: None,
        server_kind: String::new(),
        server_value: String::new(),
        caller_server: None,
        inside_tmux: false,
        attach: true,
        core: None,
        core_version: None,
        no_autostart: true,
        starter: super::Starter::Caller,
        test_pre_lock_marker: None,
    };
    let shape = super::Session {
        name: "s".to_owned(),
        mode: super::Mode::Local,
        work_dir: PathBuf::from("/w"),
        origin: PathBuf::from("/o"),
        layout: "vertical".to_owned(),
        look: crate::theme::Look::DEFAULT,
        chat: false,
        resuming: true,
        dir_created: false,
    };
    let launching = [super::Launching {
        slot: "main".to_owned(),
        name: "lead".to_owned(),
        profile: "fable5".to_owned(),
        client: None,
        binary: "claude".to_owned(),
        tool: ToolKind::Claude,
        session_id: "sid".to_owned(),
        config_home: None,
        config_home_base: None,
        launch_id: "L1".to_owned(),
        pane: "%0".to_owned(),
        command_snapshot: None,
    }];
    let watchdog = super::WatchdogFacts {
        meta_agent: false,
        sweep_sec: None,
        quota_every_secs: "300",
        idle_nudge_secs: "300",
        done_confirmations: "2",
        quota: "on",
    };
    let document =
        super::meta_document(&env, &shape, &launching, watchdog, None).expect("a resume document");
    assert!(
        document.contains("observed_model.main=Opus 5 (1M context)\n"),
        "{document}"
    );
    assert!(
        document.contains("observed_model_pin.main=fable\n"),
        "{document}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_resume_document_fixture_preserves_an_overlapping_fixture_and_cleans_its_root() {
    let (first, dir, env, ..) = resume_parts("first meta\n");
    let first_path = env.home.parent().unwrap().to_path_buf();
    std::fs::write(dir.join("sentinel"), "first").unwrap();
    let (second, other, env, ..) = resume_parts("second meta\n");
    let second_path = env.home.parent().unwrap().to_path_buf();
    println!("owned fixture roots: {first_path:?} {second_path:?}");
    assert_eq!(std::fs::read(dir.join("sentinel")).unwrap(), b"first");
    assert_eq!(std::fs::read(dir.join("meta")).unwrap(), b"first meta\n");
    assert_eq!(std::fs::read(other.join("meta")).unwrap(), b"second meta\n");
    assert_ne!(first_path, second_path);
    drop(second);
    assert!(!second_path.exists());
    assert_eq!(std::fs::read(dir.join("sentinel")).unwrap(), b"first");
    drop(first);
    assert!(!first_path.exists());
}

struct ResumeScratch(PathBuf);
impl AsRef<std::path::Path> for ResumeScratch {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for ResumeScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A resume rebuild's parts over a prior meta: the scratch root, the
/// session dir, and the launch inputs. The caller drives `meta_document`
/// itself — `resume_over` is the no-breakage shorthand.
fn resume_parts(
    meta_text: &str,
) -> (
    ResumeScratch,
    PathBuf,
    super::Env,
    super::Session,
    [super::Launching; 1],
    super::WatchdogFacts<'_>,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = (0..64)
        .find_map(|_| {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = PathBuf::from(format!("/tmp/ae-resume-{}-{n}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => Some(ResumeScratch(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => panic!("resume scratch: {e}"),
            }
        })
        .expect("an unused resume scratch root within 64 attempts");
    let home = root.as_ref().join("home");
    let dir = home.join("sessions").join("s");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("meta"), meta_text).unwrap();
    let env = super::Env {
        home: home.clone(),
        cwd: PathBuf::from("/w"),
        global: None,
        local: None,
        server_kind: String::new(),
        server_value: String::new(),
        caller_server: None,
        inside_tmux: false,
        attach: true,
        core: None,
        core_version: None,
        no_autostart: true,
        starter: super::Starter::Caller,
        test_pre_lock_marker: None,
    };
    let shape = super::Session {
        name: "s".to_owned(),
        mode: super::Mode::Local,
        work_dir: PathBuf::from("/w"),
        origin: PathBuf::from("/o"),
        layout: "vertical".to_owned(),
        look: crate::theme::Look::DEFAULT,
        chat: false,
        resuming: true,
        dir_created: false,
    };
    let launching = [super::Launching {
        slot: "main".to_owned(),
        name: "lead".to_owned(),
        profile: "fable5".to_owned(),
        client: None,
        binary: "claude".to_owned(),
        tool: ToolKind::Claude,
        session_id: "sid".to_owned(),
        config_home: None,
        config_home_base: None,
        launch_id: "L1".to_owned(),
        pane: "%0".to_owned(),
        command_snapshot: None,
    }];
    let watchdog = super::WatchdogFacts {
        meta_agent: false,
        sweep_sec: None,
        quota_every_secs: "300",
        idle_nudge_secs: "300",
        done_confirmations: "2",
        quota: "on",
    };
    (root, dir, env, shape, launching, watchdog)
}

/// A resume rebuild over a prior meta: the owned root, session dir,
/// prior bytes, and whatever the rebuild decided.
fn resume_over(meta_text: &str) -> (ResumeScratch, PathBuf, Vec<u8>, Result<String, String>) {
    let (root, dir, env, shape, launching, watchdog) = resume_parts(meta_text);
    let before = std::fs::read(dir.join("meta")).unwrap();
    let document = super::meta_document(&env, &shape, &launching, watchdog, None);
    (root, dir, before, document)
}

#[test]
fn a_resume_document_carries_a_valid_seat_dir_forward() {
    let (_root, dir, _before, document) = resume_over(
        "schema=2\nmeta_version=2\nmode=local\nsession=s\nsession_id=SID\ncreated=1\n\
             started=1\nwork_dir=/w\norigin=/o\nseat.main=lead\nprofile.main=fable5\n\
             agent_bin.main=claude\nharness_session.main=sid\nlaunch_id.main=L1\n\
             work_dir.main=/w/target\n",
    );
    let document = document.expect("a resume document");
    assert!(document.contains("work_dir.main=/w/target\n"), "{document}");
    let root = dir
        .parent()
        .and_then(|sessions| sessions.parent())
        .and_then(|home| home.parent())
        .expect("a scratch root");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_resume_document_refuses_a_malformed_seat_dir_without_mutating() {
    for bad in [
        "work_dir.main=relative\n",
        "work_dir.main=\n",
        "work_dir.main=/a\nwork_dir.main=/b\n",
    ] {
        let (_root, dir, before, document) = resume_over(&format!(
            "schema=2\nmeta_version=2\nmode=local\nsession=s\nsession_id=SID\ncreated=1\n\
                 started=1\nwork_dir=/w\norigin=/o\nseat.main=lead\nprofile.main=fable5\n\
                 agent_bin.main=claude\nharness_session.main=sid\nlaunch_id.main=L1\n{bad}"
        ));
        let why = document.expect_err("a refusal");
        assert!(why.contains("work_dir.main"), "{why}");
        // Refuse BEFORE mutate: the rebuild failed while building its
        // string, so the file still holds exactly the prior bytes.
        assert_eq!(std::fs::read(dir.join("meta")).unwrap(), before);
        let root = dir
            .parent()
            .and_then(|sessions| sessions.parent())
            .and_then(|home| home.parent())
            .expect("a scratch root");
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn a_resume_that_cannot_read_its_meta_refuses_rather_than_dropping_rows() {
    let (root, dir, env, shape, launching, watchdog) = resume_parts(
        "schema=2\nmeta_version=2\nmode=local\nsession=s\nsession_id=SID\ncreated=1\n\
             started=1\nwork_dir=/w\norigin=/o\nseat.main=lead\nprofile.main=fable5\n\
             agent_bin.main=claude\nharness_session.main=sid\nlaunch_id.main=L1\n\
             work_dir.main=/w/target\n",
    );
    std::fs::set_permissions(dir.join("meta"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let why =
        super::meta_document(&env, &shape, &launching, watchdog, None).expect_err("a refusal");
    assert!(why.contains("could not read the prior meta"), "{why}");
    std::fs::set_permissions(dir.join("meta"), std::fs::Permissions::from_mode(0o600)).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn spawned_entries_refuses_an_unreadable_meta_without_losing_rows() {
    let (root, dir, _env, _shape, _launching, _watchdog) = resume_parts(
        "schema=2\nmeta_version=2\nmode=local\nsession=s\nsession_id=SID\ncreated=1\n\
             started=1\nwork_dir=/w\norigin=/o\nseat.main=lead\nprofile.main=fable5\n\
             seat.spawned.0=scout\nprofile.spawned.0=fable5\n",
    );
    let before = std::fs::read(dir.join("meta")).unwrap();
    std::fs::set_permissions(dir.join("meta"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let Err(why) = super::spawned_entries(&dir, "s") else {
        panic!("an unreadable meta must refuse")
    };
    assert!(why.contains("cannot read the meta"), "{why}");
    // Nothing was rebuilt from nothing: the bytes are intact and the
    // spawned seat reads back whole once readable again.
    std::fs::set_permissions(dir.join("meta"), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(std::fs::read(dir.join("meta")).unwrap(), before);
    let entries = super::spawned_entries(&dir, "s").expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].slot, "spawned.0");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn solo_preflight_passes_seat_dir_damage_for_the_rebuild_to_refuse() {
    // Ordering pin: a damaged seat-dir row is NOT identity doubt, so the
    // resume preflight passes it and the REBUILD refuses with the shared
    // restore-or-retire wording — never a generic doubtful-roster line.
    let (root, dir, _env, _shape, _launching, _watchdog) = resume_parts(
        "schema=2\nmeta_version=2\nmode=local\nsession=s\nsession_id=SID\ncreated=1\n\
             started=1\nwork_dir=/w\norigin=/o\nseat.main=lead\nprofile.main=fable5\n\
             agent_bin.main=claude\nharness_session.main=sid\nlaunch_id.main=L1\n\
             work_dir.main=relative\n",
    );
    let identity = super::recorded_solo_identity(&dir).expect("preflight passes");
    assert_eq!(
        identity,
        Some(("lead".to_owned(), "fable5".to_owned())),
        "preflight must not steal the verdict"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The `@` split: a bare profile selects no client, one `@` selects
/// exactly one label, and an empty half or a second `@` is a usage error
/// naming the value.
#[test]
fn seat_override_values_split_into_profile_and_optional_client() {
    assert_eq!(
        super::split_seat_override("fablex"),
        Ok(("fablex".to_owned(), None))
    );
    assert_eq!(
        super::split_seat_override("fablex@cc-mic"),
        Ok(("fablex".to_owned(), Some("cc-mic".to_owned())))
    );
    // (An empty VALUE never reaches the split: the plan parser refuses
    // empty `--lead` / `--seat` selections before preflight.)
    for bad in ["@cc-mic", "fablex@", "@", "fablex@cc@mic"] {
        let line = super::split_seat_override(bad).expect_err("usage");
        assert!(
            line.contains(bad) && line.contains("<profile>@<client>"),
            "{line:?}"
        );
    }
    // No `@` at all is never an error, even for a value no config names:
    // existence is the resolver's question, not the split's.
    assert_eq!(
        super::split_seat_override("missing"),
        Ok(("missing".to_owned(), None))
    );
}

fn override_cfg() -> crate::config::IdentityConfig {
    crate::config::parse_identity(
        "[clients]\n\
             claude = claude\n\
             cc-mic = claude config_home=$HOME/.claude-mic\n\
             cc-other = claude config_home=$HOME/.claude-other\n\
             codex = codex\n\
             u1 = sleep-u1\n\
             u2 = sleep-u2\n\
             [profiles]\n\
             fablex = \"claude --model fable --effort xhigh\"\n\
             solx = \"codex -m sol\"\n\
             weird1 = \"sleep-u1 --x\"\n\
             weird2 = \"sleep-u2 --x\"\n\
             [roster]\n\
             lead = fablex\n\
             [workspace]\n\
             main = lead\n",
    )
    .expect("readable override config")
}

/// R1: an override may change the account, never the harness adapter —
/// both sides must be the SAME KNOWN harness, and `Unknown` on either
/// side refuses. Gating on `from_binary_name` would let the two-unknown
/// pair through, because any two unknowns compare equal there.
#[test]
fn client_override_resolves_same_adapter_and_refuses_anything_else() {
    let cfg = override_cfg();
    let home = PathBuf::from("/Users/a");
    let (command, parsed) = super::resolve_client_override(
        &cfg,
        "lead",
        "fablex",
        "cc-mic",
        Some(&home),
        "fablex, solx, weird1, weird2",
        "claude, cc-mic, cc-other, codex, u1, u2",
    )
    .expect("same-adapter override resolves");
    assert_eq!(
        command.as_str(),
        "CLAUDE_CONFIG_DIR='/Users/a/.claude-mic' claude --model fable --effort xhigh"
    );
    assert_eq!(command.client_label(), Some("cc-mic"));
    assert_eq!(parsed.binary, "claude");
    // Cross-adapter: both values named, duplicate profile as the way out.
    let line = match super::resolve_client_override(
        &cfg,
        "lead",
        "fablex",
        "codex",
        Some(&home),
        "fablex",
        "codex",
    ) {
        Err(super::SeatOverrideRefusal::Usage(line)) => line,
        other => panic!("cross-adapter must refuse Usage, got {other:?}"),
    };
    assert!(
        line.contains("fablex@codex")
            && line.contains("claude")
            && line.contains("codex")
            && line.contains("never the harness adapter")
            && line.contains("duplicate profile"),
        "{line:?}"
    );
    // Unknown on EITHER side refuses — including unknown on BOTH.
    for (profile, label) in [
        ("weird1", "u2"),
        ("weird2", "u1"),
        ("fablex", "u1"),
        ("weird1", "cc-mic"),
    ] {
        let refused = super::resolve_client_override(
            &cfg,
            "lead",
            profile,
            label,
            Some(&home),
            "fablex",
            "codex",
        );
        assert!(
            matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
            "{profile}@{label} must refuse, got {refused:?}"
        );
    }
    // Unknown sides are named as unrecognized binaries, never as equal.
    let line = match super::resolve_client_override(
        &cfg,
        "lead",
        "weird1",
        "u2",
        Some(&home),
        "fablex",
        "codex",
    ) {
        Err(super::SeatOverrideRefusal::Usage(line)) => line,
        other => panic!("two unknowns must refuse, got {other:?}"),
    };
    assert!(
        line.contains("unrecognized binary 'sleep-u1'")
            && line.contains("unrecognized binary 'sleep-u2'"),
        "{line:?}"
    );
}

#[test]
fn client_override_errors_mirror_the_unknown_profile_shape() {
    let cfg = override_cfg();
    let home = PathBuf::from("/Users/a");
    match super::resolve_client_override(
        &cfg,
        "lead",
        "missing",
        "cc-mic",
        Some(&home),
        "fablex, solx",
        "cc-mic, codex",
    ) {
        Err(super::SeatOverrideRefusal::Usage(line)) => assert_eq!(
            line,
            "Error: unknown profile 'missing' in --seat. Known profiles: fablex, solx."
        ),
        other => panic!("unknown profile must refuse Usage, got {other:?}"),
    }
    match super::resolve_client_override(
        &cfg,
        "lead",
        "fablex",
        "ghost",
        Some(&home),
        "fablex, solx",
        "cc-mic, codex",
    ) {
        Err(super::SeatOverrideRefusal::Usage(line)) => assert_eq!(
            line,
            "Error: unknown client 'ghost' in --seat. Known clients: cc-mic, codex."
        ),
        other => panic!("unknown client must refuse Usage, got {other:?}"),
    }
}

fn recorded_entry(meta: &str) -> crate::meta::RosterEntry {
    crate::meta::Meta::parse(meta).roster().to_vec().remove(0)
}

/// R2b at the launch boundary: `Missing` is no override, `Label` needs
/// its label live in the current config, and `Invalid` fails closed.
/// Only the absent label names the restore remedy.
#[test]
fn recorded_client_states_resolve_or_refuse_with_their_remedy() {
    let cfg = override_cfg();
    let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\n");
    assert_eq!(
        super::recorded_selection("s", &cfg, &entry),
        Ok(super::RecordedSelection::Missing)
    );
    let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\nclient.main=cc-mic\n");
    assert_eq!(
        super::recorded_selection("s", &cfg, &entry),
        Ok(super::RecordedSelection::Label("cc-mic".to_owned()))
    );
    let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\nclient.main=ghost\n");
    let line = super::recorded_selection("s", &cfg, &entry).expect_err("absent refuses");
    assert!(
        line.contains("'ghost'")
            && line.contains("restore the 'ghost' client")
            && line.contains("ae end s"),
        "{line:?}"
    );
    for meta in [
        "seat.main=lead\nprofile.main=fablex\nclient.main=\n",
        "seat.main=lead\nprofile.main=fablex\nclient.main=cc mic\n",
        "seat.main=lead\nprofile.main=fablex\nclient.main=cc-mic\nclient.main=cc-other\n",
    ] {
        let entry = recorded_entry(meta);
        let line = super::recorded_selection("s", &cfg, &entry).expect_err("invalid refuses");
        assert!(
            line.contains("client.main")
                && line.contains("fix the meta row")
                && line.contains("ae end s"),
            "{meta:?} -> {line:?}"
        );
    }
}

/// R4b: once `Label(l)` is recorded, ONLY `l` is accepted. A different
/// label refuses whatever its facts, and a bare profile cannot drop the
/// row by saying nothing.
#[test]
fn recorded_label_accepts_only_its_exact_label() {
    let cfg = override_cfg();
    let home = PathBuf::from("/Users/a");
    let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\nclient.main=cc-mic\n");
    let command = cfg
        .command_with_client("fablex", "cc-mic", Some(&home))
        .expect("override resolves")
        .command;
    // The exact label, with no recorded store yet: first start proceeds.
    super::refuse_recorded_client_conflict(
        "s",
        "lead",
        "fablex",
        Some("cc-mic"),
        &entry,
        &cfg,
        &command,
        ToolKind::Claude,
        Some(&home),
    )
    .expect("exact label proceeds");
    // A different label refuses, naming the recorded one and the profile
    // spelling that would satisfy write-once.
    let line = match super::refuse_recorded_client_conflict(
        "s",
        "lead",
        "fablex",
        Some("cc-other"),
        &entry,
        &cfg,
        &command,
        ToolKind::Claude,
        Some(&home),
    ) {
        Err(super::SeatOverrideRefusal::Usage(line)) => line,
        other => panic!("different label must refuse, got {other:?}"),
    };
    assert!(
        line.contains("'cc-mic'")
            && line.contains("'cc-other'")
            && line.contains("write-once")
            && line.contains("fablex@cc-mic"),
        "{line:?}"
    );
    // Identical facts today change nothing: the equality cannot cross a
    // config edit, so even a twin store refuses by label.
    let twin = crate::config::parse_identity(
        "[clients]\ncc-mic = claude config_home=/twin\ncc-twin = claude config_home=/twin\n\
             [profiles]\nfablex = \"claude --model fable\"\n\
             [roster]\nlead = fablex\n[workspace]\nmain = lead\n",
    )
    .expect("twin config");
    let twin_command = twin
        .command_with_client("fablex", "cc-twin", None)
        .expect("twin resolves")
        .command;
    let refused = super::refuse_recorded_client_conflict(
        "s",
        "lead",
        "fablex",
        Some("cc-twin"),
        &entry,
        &twin,
        &twin_command,
        ToolKind::Claude,
        None,
    );
    assert!(
        matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
        "a twin store under another label still refuses, got {refused:?}"
    );
    // A bare profile cannot drop the row.
    let plain = cfg
        .command("fablex", Some(&home))
        .expect("profile resolves")
        .expect("profile exists");
    let line = match super::refuse_recorded_client_conflict(
        "s",
        "lead",
        "astrax",
        None,
        &entry,
        &cfg,
        &plain,
        ToolKind::Claude,
        Some(&home),
    ) {
        Err(super::SeatOverrideRefusal::Usage(line)) => line,
        other => panic!("bare re-pair must refuse, got {other:?}"),
    };
    assert!(
        line.contains("'cc-mic'") && line.contains("astrax@cc-mic"),
        "{line:?}"
    );
    // No recorded row and no selection: the legacy path is untouched.
    let legacy = recorded_entry("seat.main=lead\nprofile.main=fablex\n");
    super::refuse_recorded_client_conflict(
        "s",
        "lead",
        "astrax",
        None,
        &legacy,
        &cfg,
        &plain,
        ToolKind::Claude,
        Some(&home),
    )
    .expect("legacy bare override proceeds");
}

/// The R4 call site, not just the comparison: a FIRST override on a seat
/// that never recorded one still re-takes the triple against the recorded
/// store. Skipping the call would let a conflicting store through.
#[test]
fn first_override_against_a_recorded_store_retakes_the_triple() {
    let home = PathBuf::from("/home-op");
    let cfg = crate::config::parse_identity(
        "[clients]\ncc-a = claude config_home=/store-a\ncc-b = claude config_home=/store-b\n\
             [profiles]\nf = \"claude --model fable\"\n\
             [roster]\nlead = f\n[workspace]\nmain = lead\n",
    )
    .expect("two-store config");
    let entry = recorded_entry("seat.main=lead\nprofile.main=f\nconfig_home.main=/store-a\n");
    // Same store: proceeds, and the launch records the row from then on.
    let matching = cfg
        .command_with_client("f", "cc-a", Some(&home))
        .expect("matching resolves")
        .command;
    super::refuse_recorded_client_conflict(
        "s",
        "lead",
        "f",
        Some("cc-a"),
        &entry,
        &cfg,
        &matching,
        ToolKind::Claude,
        Some(&home),
    )
    .expect("matching first override proceeds");
    // Another store: refuses, naming both.
    let conflicting = cfg
        .command_with_client("f", "cc-b", Some(&home))
        .expect("conflicting resolves")
        .command;
    let line = match super::refuse_recorded_client_conflict(
        "s",
        "lead",
        "f",
        Some("cc-b"),
        &entry,
        &cfg,
        &conflicting,
        ToolKind::Claude,
        Some(&home),
    ) {
        Err(super::SeatOverrideRefusal::Usage(line)) => line,
        other => panic!("conflicting first override must refuse, got {other:?}"),
    };
    assert!(
        line.contains("/store-a") && line.contains("/store-b"),
        "{line:?}"
    );
}

/// R4: the recorded store triple (MODE + PATH + BASE) is re-taken against
/// the current resolution at every launch. Path alone is not identity: an
/// implicit and an explicit row over one path refuse each other.
#[test]
fn store_conflict_compares_mode_path_and_base_never_path_alone() {
    let home = PathBuf::from("/home-op");
    let mic = |store: &str| {
        crate::config::parse_identity(&format!(
            "[clients]\ncc = claude config_home={store}\n\
                 [profiles]\nf = \"claude --model fable\"\n\
                 [roster]\nlead = f\n[workspace]\nmain = lead\n",
        ))
        .expect("mic config")
    };
    let resolve = |cfg: &crate::config::IdentityConfig| {
        cfg.command_with_client("f", "cc", Some(&home))
            .expect("override resolves")
            .command
    };
    // Matching explicit store proceeds.
    let cfg = mic("/store-a");
    let command = resolve(&cfg);
    let entry = recorded_entry("seat.main=lead\nprofile.main=f\nconfig_home.main=/store-a\n");
    super::refuse_store_conflict(
        "s",
        "lead",
        "f@cc",
        &entry,
        &command,
        ToolKind::Claude,
        Some(&home),
    )
    .expect("matching store proceeds");
    // A definition that moved under the label refuses, naming both.
    let moved = mic("/store-b");
    let moved_command = resolve(&moved);
    let line = match super::refuse_store_conflict(
        "s",
        "lead",
        "f@cc",
        &entry,
        &moved_command,
        ToolKind::Claude,
        Some(&home),
    ) {
        Err(super::SeatOverrideRefusal::Usage(line)) => line,
        other => panic!("moved definition must refuse, got {other:?}"),
    };
    assert!(
        line.contains("/store-a")
            && line.contains("/store-b")
            && line.contains("cannot move a retained conversation")
            && line.contains("ae end s"),
        "{line:?}"
    );
    // Implicit versus explicit over ONE path: different accounts.
    let implicit_cfg = crate::config::parse_identity(
        "[clients]\ncc = claude\n[profiles]\nf = \"claude --model fable\"\n\
             [roster]\nlead = f\n[workspace]\nmain = lead\n",
    )
    .expect("implicit config");
    let implicit_command = implicit_cfg
        .command_with_client("f", "cc", Some(&home))
        .expect("implicit override resolves")
        .command;
    let entry =
        recorded_entry("seat.main=lead\nprofile.main=f\nconfig_home.main=/home-op/.claude\n");
    let refused = super::refuse_store_conflict(
        "s",
        "lead",
        "f@cc",
        &entry,
        &implicit_command,
        ToolKind::Claude,
        Some(&home),
    );
    assert!(
        matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
        "explicit recorded versus implicit current refuses, got {refused:?}"
    );
    // The same path in the same mode proceeds.
    let entry = recorded_entry(
        "seat.main=lead\nprofile.main=f\nconfig_home.main=implicit:/home-op/.claude\nconfig_home_base.main=/home-op\n",
    );
    super::refuse_store_conflict(
        "s",
        "lead",
        "f@cc",
        &entry,
        &implicit_command,
        ToolKind::Claude,
        Some(&home),
    )
    .expect("matching implicit store proceeds");
    // ... but not under another base.
    let elsewhere = PathBuf::from("/home-elsewhere");
    let refused = super::refuse_store_conflict(
        "s",
        "lead",
        "f@cc",
        &entry,
        &implicit_command,
        ToolKind::Claude,
        Some(&elsewhere),
    );
    assert!(
        matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
        "same path under another HOME refuses, got {refused:?}"
    );
}

/// The R4 edges: first start proceeds, a current `Unknown` proves no
/// conflict (`_run` retains with a notice), and recorded `Unknown` /
/// `Invalid` / mismatched `Absent` fail closed.
#[test]
fn store_conflict_edges_fail_closed_except_first_start_and_unknown_current() {
    let home = PathBuf::from("/Users/a");
    let cfg = override_cfg();
    let command = cfg
        .command_with_client("fablex", "cc-mic", Some(&home))
        .expect("override resolves")
        .command;
    // First start: no recorded store, the override is recorded normally.
    let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\nclient.main=cc-mic\n");
    super::refuse_store_conflict(
        "s",
        "lead",
        "fablex@cc-mic",
        &entry,
        &command,
        ToolKind::Claude,
        Some(&home),
    )
    .expect("first start proceeds");
    // Recorded damage fails closed.
    for meta in [
        "seat.main=lead\nprofile.main=fablex\nconfig_home.main=unknown\n",
        "seat.main=lead\nprofile.main=fablex\nconfig_home.main=\n",
    ] {
        let entry = recorded_entry(meta);
        assert!(
            super::refuse_store_conflict(
                "s",
                "lead",
                "fablex@cc-mic",
                &entry,
                &command,
                ToolKind::Claude,
                Some(&home),
            )
            .is_err(),
            "{meta:?} fails closed"
        );
    }
    // Recorded Absent matches only Absent.
    let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\nconfig_home.main=absent\n");
    let refused = super::refuse_store_conflict(
        "s",
        "lead",
        "fablex@cc-mic",
        &entry,
        &command,
        ToolKind::Claude,
        Some(&home),
    );
    assert!(
        matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
        "absent recorded versus explicit current refuses, got {refused:?}"
    );
    // A current Unknown (a pane-only relative assignment the launcher
    // cannot resolve) proves no conflict and proceeds. The override
    // client is implicit, so the pair takes no conflict — the Unknown
    // comes from the profile's own unresolvable assignment.
    let relative = crate::config::parse_identity(
        "[clients]\nplain = claude\n\
             [profiles]\nr = \"CLAUDE_CONFIG_DIR=relative/path claude --model fable\"\n\
             [roster]\nlead = r\n[workspace]\nmain = lead\n",
    )
    .expect("relative config");
    let relative_command = relative
        .command_with_client("r", "plain", Some(&home))
        .expect("relative override resolves")
        .command;
    let entry =
        recorded_entry("seat.main=lead\nprofile.main=r\nconfig_home.main=/Users/a/.claude\n");
    super::refuse_store_conflict(
        "s",
        "lead",
        "r@plain",
        &entry,
        &relative_command,
        ToolKind::Claude,
        Some(&home),
    )
    .expect("unknown current proceeds; _run retains");
}

/// The split every `profile@client` spelling shares: no `@` is a bare
/// profile, one `@` splits, and a malformed value is `None` for the
/// caller to word.
#[test]
fn profile_client_values_split_once_or_are_malformed() {
    assert_eq!(
        super::split_profile_client("opus"),
        Some(("opus".to_owned(), None))
    );
    assert_eq!(
        super::split_profile_client("opus@cc-mic"),
        Some(("opus".to_owned(), Some("cc-mic".to_owned())))
    );
    for bad in ["@cc-mic", "opus@", "@", "opus@cc@mic", "a@b@"] {
        assert_eq!(super::split_profile_client(bad), None, "{bad}");
    }
}

/// A spawn takes the launch's own gate: the same-harness pair resolves to
/// the override command, a cross-harness pair names both sides, and a
/// first start on a store the launcher cannot resolve refuses (F6).
#[test]
fn a_spawn_pair_takes_the_adapter_gate_and_the_first_start_store_proof() {
    let cfg = override_cfg();
    let home = PathBuf::from("/Users/a");
    let (command, parsed) =
        super::resolve_spawn_pair(&cfg, "scout", "fablex", "cc-mic", Some(&home))
            .expect("a same-harness pair resolves");
    assert_eq!(
        command.as_str(),
        "CLAUDE_CONFIG_DIR='/Users/a/.claude-mic' claude --model fable --effort xhigh"
    );
    assert_eq!(command.client_label(), Some("cc-mic"));
    assert_eq!(parsed.binary, "claude");
    let crossed = super::resolve_spawn_pair(&cfg, "scout", "fablex", "codex", Some(&home))
        .expect_err("a cross-harness pair refuses");
    assert!(
        crossed.contains("'scout'")
            && crossed.contains("fablex@codex")
            && crossed.contains("never the harness adapter"),
        "{crossed}"
    );
    let blind = crate::config::parse_identity(
        "[clients]\nplain = claude\n\
             [profiles]\nvarp = \"CLAUDE_CONFIG_DIR=rel/store claude --model fable\"\n\
             [roster]\nlead = varp\n[workspace]\nmain = lead\n",
    )
    .expect("readable blind config");
    let unknown = super::resolve_spawn_pair(&blind, "scout", "varp", "plain", Some(&home))
        .expect_err("an unverifiable store refuses at a first start");
    assert!(
        unknown.contains("varp@plain") && unknown.contains("unknown conversation store"),
        "{unknown}"
    );
}

fn restored(client: Option<&str>, profile: &str) -> super::Spawned {
    super::Spawned {
        slot: "spawned.0".to_owned(),
        name: "scout".to_owned(),
        profile: profile.to_owned(),
        client: client.map(ToOwned::to_owned),
        binary: "claude".to_owned(),
        harness_session: String::new(),
        config_home: None,
        config_home_base: None,
    }
}

/// A restored spawned seat resolves through the client it RECORDED: a bare
/// seat takes the profile's own command, a recorded label the override, an
/// unconfigured profile keeps the seat verbatim, and a recorded label no
/// `[clients]` row names refuses with its remedy.
#[test]
fn a_restored_spawned_seat_resolves_through_its_recorded_client() {
    let cfg = override_cfg();
    let home = PathBuf::from("/Users/a");
    let bare = super::spawned_command(&cfg, &restored(None, "fablex"), Some(&home))
        .expect("a bare seat resolves")
        .expect("a configured profile");
    assert_eq!(bare.as_str(), "claude --model fable --effort xhigh");
    let pair = super::spawned_command(&cfg, &restored(Some("cc-mic"), "fablex"), Some(&home))
        .expect("a recorded client resolves")
        .expect("a configured profile");
    assert_eq!(
        pair.as_str(),
        "CLAUDE_CONFIG_DIR='/Users/a/.claude-mic' claude --model fable --effort xhigh"
    );
    assert_eq!(
        super::spawned_command(&cfg, &restored(Some("cc-mic"), "gone"), Some(&home)),
        Ok(None),
        "an unconfigured profile keeps the seat verbatim"
    );
    assert_eq!(
        super::spawned_command(&cfg, &restored(None, "gone"), Some(&home)),
        Ok(None)
    );
    let missing = super::spawned_command(&cfg, &restored(Some("ghost"), "fablex"), None)
        .expect_err("a recorded client nothing names refuses");
    assert!(
        missing.contains("'ghost'") && missing.contains("restore the 'ghost' client"),
        "{missing}"
    );
    let mixed = crate::config::parse_identity(
        "[clients]\nclaude = claude\n[profiles]\nmulti = \"claude; touch m\"\n\
             [roster]\nlead = multi\n[workspace]\nmain = lead\n",
    )
    .expect("a two-command profile still parses as a config");
    let refused = super::spawned_command(&mixed, &restored(Some("claude"), "multi"), None)
        .expect_err("a two-command profile refuses on restore");
    assert!(refused.contains("Nothing was resumed"), "{refused}");
}

/// F6: a first start whose override resolves `Unknown` refuses — nothing
/// retained means nothing stranded, and proceeding would ignore the
/// explicit flag.
#[test]
fn unresolvable_first_start_refuses_naming_the_override_and_why() {
    let home = PathBuf::from("/Users/a");
    let entry = recorded_entry("seat.main=lead\nprofile.main=varp\n");
    let blind = crate::config::parse_identity(
        "[clients]\nplain = claude\n\
             [profiles]\nvarp = \"CLAUDE_CONFIG_DIR=rel/store claude --model fable\"\n\
             [roster]\nlead = varp\n[workspace]\nmain = lead\n",
    )
    .expect("readable blind config");
    let blind_command = blind
        .command_with_client("varp", "plain", Some(&home))
        .expect("blind override resolves")
        .command;
    let line = match super::refuse_store_conflict(
        "s",
        "lead",
        "varp@plain",
        &entry,
        &blind_command,
        ToolKind::Claude,
        Some(&home),
    ) {
        Err(super::SeatOverrideRefusal::Usage(line)) => line,
        other => panic!("unresolvable first start must refuse, got {other:?}"),
    };
    assert!(
        line.contains("varp@plain")
            && line.contains("unknown conversation store")
            && line.contains("not an absolute path"),
        "{line:?}"
    );
}

/// One read showing the human-only prompt asking `question`.
fn asking(question: &str) -> super::ReadyRead {
    super::ReadyRead::Unready(Some(crate::watchdog::HumanPrompt {
        kind: crate::watchdog::DialogKind::FolderTrust,
        warnings: Vec::new(),
        question: question.to_owned(),
        keys: "Enter to confirm · Esc to cancel".to_owned(),
        trust: None,
        dismiss: false,
    }))
}

/// Fold `reads`, counting how many the wait actually TOOK: the whole
/// difference between the two callers is how far it reads.
fn settled(reads: Vec<super::ReadyRead>, on: super::OnHumanPrompt) -> (super::Readiness, usize) {
    let taken = std::cell::Cell::new(0);
    let counted = reads.into_iter().inspect(|_| taken.set(taken.get() + 1));
    let verdict = super::settle(counted, on);
    (verdict, taken.get())
}

fn blocked(question: &str) -> super::Readiness {
    super::Readiness::Blocked(crate::watchdog::HumanPrompt {
        kind: crate::watchdog::DialogKind::FolderTrust,
        warnings: Vec::new(),
        question: question.to_owned(),
        keys: "Enter to confirm · Esc to cancel".to_owned(),
        trust: None,
        dismiss: false,
    })
}

/// The caller's choice: a single-seat verb gives the pane back on the
/// second identical read; a launch reads every chunk and names the prompt
/// only when it still holds at the end.
#[test]
fn fail_fast_stops_on_the_second_same_prompt_and_wait_reads_every_chunk() {
    use super::OnHumanPrompt::{FailFast, Wait};
    let five = || (0..5).map(|_| asking("trust?")).collect::<Vec<_>>();
    assert_eq!(settled(five(), FailFast), (blocked("trust?"), 2));
    assert_eq!(settled(five(), Wait), (blocked("trust?"), 5));
    // Held mid-wait, gone at the end: a launch names nothing.
    let mut cleared = five();
    cleared.push(super::ReadyRead::Unready(None));
    assert_eq!(settled(cleared, Wait), (super::Readiness::NotReady, 6));
}

/// STABLE is the same question on two CONSECUTIVE reads, never one; and
/// readiness wins whenever it comes, a prompt the human answered included.
#[test]
fn only_the_same_prompt_twice_in_a_row_is_stable_and_readiness_always_wins() {
    use super::OnHumanPrompt::{FailFast, Wait};
    use super::ReadyRead::{Ready, Unready};
    for on in [FailFast, Wait] {
        assert_eq!(
            settled(vec![asking("a?")], on).0,
            super::Readiness::NotReady
        );
        let gap = vec![asking("a?"), Unready(None), asking("a?")];
        assert_eq!(settled(gap, on).0, super::Readiness::NotReady, "{on:?}");
        let other = vec![asking("a?"), asking("b?")];
        assert_eq!(settled(other, on).0, super::Readiness::NotReady, "{on:?}");
        let answered = vec![asking("a?"), Ready, asking("a?"), asking("a?")];
        assert_eq!(
            settled(answered, on),
            (super::Readiness::Ready, 2),
            "{on:?}"
        );
    }
}

#[test]
fn a_chunked_wait_keeps_the_whole_launch_budget_for_either_input_model() {
    // `deliver::wait_input_ready` sleeps once per poll for a modelled pane
    // and once per poll after the seed capture for an unmodelled one: both
    // prompt rows' tools must wait the unchunked `LAUNCH_READY_POLLS`.
    let chunks = super::READY_CHUNKS;
    for tool in [crate::tool::ToolKind::Claude, crate::tool::ToolKind::Agy] {
        let model = tool.adapter().input.model;
        let polls = super::chunk_polls(model);
        let sleeps = if model.is_modelled() {
            polls
        } else {
            polls - 1
        };
        assert_eq!(chunks * sleeps, super::LAUNCH_READY_POLLS, "{tool:?}");
    }
}

#[test]
fn an_unattended_expectation_has_no_client_to_prove() {
    // An unattended start captured no clicker: the narrow attachment proof
    // refuses it, so a failure report never reaches a client that is not there.
    for state in [
        ExpectedState::AbsentCanonical,
        ExpectedState::StoppedSession,
    ] {
        let expected = ExpectedLaunch::unattended(state, 4);
        assert_eq!(
            expected.check_attachment(),
            Err("an unattended start has no client".to_owned())
        );
    }
}

#[test]
fn a_clicker_whose_server_no_longer_answers_is_refused() {
    // The captured server's socket does not exist: nothing answers there, so
    // the clicker cannot be proven and no message may be sent to it.
    let server = ServerId::Selected(crate::meta::Selector::Socket(std::path::PathBuf::from(
        "/nonexistent-ae-socket-dir/server",
    )));
    let expected = ExpectedLaunch::new(
        ExpectedState::AbsentCanonical,
        server,
        "1".to_owned(),
        "2".to_owned(),
        "/dev/ttys004".to_owned(),
        "3".to_owned(),
        4,
    );
    assert_eq!(
        expected.check_attachment(),
        Err("the invoking tmux server no longer answers".to_owned())
    );
}

/// `--from` and compact seed the child's `goal=` from the archive's `source_goal` (#72).
mod inherited_goal_spec {
    use super::super::{FromProof, announce_inherited_goal, meta_document};
    use super::resume_parts;

    const AID: &str = "44444444-4444-4444-4444-444444444444";

    /// A fresh launch of session `s` that inherits archive `AID`, whose meta is `archive_meta`
    /// and whose child dir holds `child_meta` already (empty: nothing). Answers the document.
    fn from_document(archive_meta: &str, child_meta: &str) -> String {
        let (root, dir, env, mut shape, launching, watchdog) = resume_parts(child_meta);
        shape.resuming = false;
        if child_meta.is_empty() {
            std::fs::remove_file(dir.join("meta")).unwrap();
        }
        let archive = env.home.join("archive").join(AID);
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(archive.join("meta"), archive_meta).unwrap();
        let proof = FromProof {
            id: AID.to_owned(),
            handover: "1".to_owned(),
            pending: "0".to_owned(),
        };
        let document = meta_document(&env, &shape, &launching, watchdog, Some(&proof))
            .expect("a --from document");
        drop(root);
        document
    }

    fn goal_rows(document: &str) -> Vec<&str> {
        document
            .lines()
            .filter(|line| line.starts_with("goal="))
            .collect()
    }

    #[test]
    fn a_from_child_starts_with_the_source_goal_verbatim() {
        let document = from_document(
            "archive_version=1\nsource_session=s\nsource_goal=ship a=b and c\nhandover_count=1\n",
            "",
        );
        assert_eq!(goal_rows(&document), ["goal=ship a=b and c"], "{document}");
        assert!(
            document.contains(&format!("parent_archive_id={AID}\n")),
            "lineage is still recorded: {document}"
        );
    }

    #[test]
    fn an_empty_or_absent_source_goal_seeds_nothing() {
        for archive_meta in [
            "archive_version=1\nsource_goal=\n",
            "archive_version=1\nsource_goal=\x07\x1b\n",
            "archive_version=1\nsource_goal=\t \n",
            "archive_version=1\n",
        ] {
            let document = from_document(archive_meta, "");
            assert!(
                goal_rows(&document).is_empty(),
                "{archive_meta:?} seeds nothing: {document}"
            );
        }
    }

    #[test]
    fn a_source_goal_is_seeded_as_one_printable_line() {
        let document = from_document(
            "archive_version=1\nsource_goal=ship\x07 it\t\x1bnow\r\n",
            "",
        );
        assert_eq!(goal_rows(&document), ["goal=ship it now"], "{document:?}");
    }

    #[test]
    fn a_goal_the_child_already_holds_is_not_replaced_by_the_archives() {
        let document = from_document(
            "archive_version=1\nsource_goal=from the archive\n",
            "session=s\ngoal=set for the child\n",
        );
        assert_eq!(
            goal_rows(&document),
            ["goal=set for the child"],
            "{document}"
        );
    }

    /// What `announce_inherited_goal` writes for a child named `child` whose meta is
    /// `child_meta`, inheriting an archive made from the session `source_session`, and
    /// whether the child then holds an event journal.
    fn announced(child_meta: &str, source_session: &str, child: &str) -> (String, bool) {
        let (_root, dir, env, ..) = resume_parts(child_meta);
        let archive = env.home.join("archive").join(AID);
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(
            archive.join("meta"),
            format!("archive_version=1\nsource_session={source_session}\n"),
        )
        .unwrap();
        let mut err = Vec::new();
        announce_inherited_goal(&dir, &env.home.join("archive"), AID, child, &mut err);
        (
            String::from_utf8(err).unwrap(),
            dir.join("events.jsonl").exists(),
        )
    }

    #[test]
    fn a_child_under_another_name_is_told_where_its_goal_came_from_and_journals_nothing() {
        let (said, journalled) = announced("session=s\ngoal=ship it\n", "old", "s");
        assert_eq!(
            said,
            format!("goal inherited from archive {AID}; use goal --clear to drop\n")
        );
        assert!(
            !journalled,
            "the seed writes no goal event: a seat whose tool stamps no launch_time would be told it as a change"
        );
    }

    #[test]
    fn a_child_under_the_archives_own_name_takes_its_goal_without_a_word() {
        let (said, journalled) = announced("session=s\ngoal=ship it\n", "s", "s");
        assert_eq!(said, "");
        assert!(!journalled);
    }

    #[test]
    fn a_child_with_no_goal_says_nothing() {
        for meta in ["session=s\n", "session=s\ngoal=\n"] {
            let (said, journalled) = announced(meta, "old", "s");
            assert_eq!(said, "", "{meta:?}");
            assert!(!journalled, "{meta:?}");
        }
    }

    #[test]
    fn a_launch_with_no_parent_seeds_no_goal() {
        let (_root, dir, env, mut shape, launching, watchdog) = resume_parts("");
        shape.resuming = false;
        std::fs::remove_file(dir.join("meta")).unwrap();
        let archive = env.home.join("archive").join(AID);
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(archive.join("meta"), "source_goal=unrelated\n").unwrap();
        let document = meta_document(&env, &shape, &launching, watchdog, None).expect("a document");
        assert!(goal_rows(&document).is_empty(), "{document}");
    }
}

/// A start row that cannot be appended rolls the session back, but only a
/// strict absence proof lets the record go: on a server ae cannot reach, the
/// kill fails, the line says so with the stop that finds the session, and the
/// record this attempt created stays for that stop.
#[test]
fn an_unjournaled_start_keeps_the_record_when_the_rollback_cannot_prove_the_session_gone() {
    let root = scratch("unjournaled-unproven");
    let dir = root.join("s");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("meta"), "session=s\n").unwrap();
    let shape = super::Session {
        name: "s".to_owned(),
        mode: super::Mode::Local,
        work_dir: PathBuf::from("/w"),
        origin: PathBuf::from("/o"),
        layout: "vertical".to_owned(),
        look: crate::theme::Look::DEFAULT,
        chat: false,
        resuming: false,
        dir_created: true,
    };
    let server = ServerId::Selected(crate::meta::Selector::Socket(PathBuf::from(
        "/nonexistent-ae-socket-dir/server",
    )));
    let mut err = Vec::new();
    let code = super::rollback_unjournaled(&shape, &dir, &server, "the journal is full", &mut err)
        .expect("a vector takes every line");
    assert_eq!(code, crate::state::EXIT_FAILED);
    assert_eq!(
        String::from_utf8_lossy(&err),
        "ae: launch failed — the start of 's' could not be journaled (the journal is full), and \
         the rollback could not confirm it stopped (the kill failed). Stop it with: ae stop s\n"
    );
    assert!(
        dir.join("meta").is_file(),
        "the record this attempt created stays for the stop the line names"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn autostart_counts_a_live_session_only_when_its_stamp_names_the_record() {
    use crate::tmux::OptionReading;
    const SAVED: &str = "0199c0de-3333-4890-abcd-ef0123456789";
    let foreign = super::preflight::foreign_incarnation;
    let meta = format!("name=orchestrator\nsession_id={SAVED}\n");
    let meta = meta.as_bytes();
    assert_eq!(
        foreign(meta, &OptionReading::Set(SAVED.to_ascii_uppercase())),
        None
    );
    let vacant = foreign(meta, &OptionReading::Vacant).expect("an unstamped session is foreign");
    assert!(
        vacant.contains("no ae session uuid") && vacant.contains(SAVED),
        "{vacant}"
    );
    let other = "0199c0de-4444-4890-abcd-ef0123456789";
    let mismatch = foreign(meta, &OptionReading::Set(other.to_owned())).expect("another uuid");
    assert!(
        mismatch.contains(other) && mismatch.contains(SAVED),
        "{mismatch}"
    );
    assert!(foreign(meta, &OptionReading::Unknown).is_some());
    let doubled = format!("session_id={SAVED}\nsession_id={SAVED}\n");
    assert!(foreign(doubled.as_bytes(), &OptionReading::Set(SAVED.to_owned())).is_some());
    assert!(foreign(b"name=orchestrator\n", &OptionReading::Vacant).is_some());
}
