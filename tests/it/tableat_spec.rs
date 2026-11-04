//! Frozen table-at characterization, base c1f4549f (#300).
//! Oracle: docs/reference/commands.md `ae list` for semantic cells; otherwise
//! golden bytes captured from the unrefactored base, deliberately behavior pins.
//! Character counts (including CJK), raw origin spaces, and caller row order are
//! existing behavior, not new display-width or sanitization requirements.

use crate::attention::Reason;
use crate::digest::{AgentEntry, Declaration, HarnessClash, SessionEntry, Status};
use crate::harness_state::HarnessState;
use crate::time::Timestamp;
use crate::watchdog::{DoneProgress, WaitProgress};

const NOW: i64 = 2_000_000;

fn seat(name: &str, state: Option<&str>) -> AgentEntry {
    AgentEntry {
        reference: name.into(),
        name: name.into(),
        alias: "profile".into(),
        state: state.map(str::to_owned),
        ..AgentEntry::default()
    }
}

fn made(reason: &str, at: Option<i64>) -> Declaration {
    Declaration::Made {
        reason: reason.into(),
        at,
        on: None,
        overdue: false,
        missing: false,
        moved: None,
    }
}

fn pin(tag: &str, sessions: &[SessionEntry], expected: &str) {
    let refs = sessions.iter().collect::<Vec<_>>();
    let actual = super::table_at(&refs, Timestamp::from_epoch(NOW));
    assert_eq!(actual, expected, "{tag}: whole table bytes");
}

#[test]
fn tableat_session_rows_keep_order_markers_origin_and_summary() {
    let mut live = SessionEntry::new("live", Status::Running);
    live.origin = Some("/work/live ".into());
    live.attention = Some(Reason::Blocked);
    live.unanswered = Some(2);
    live.wait_end = Some(crate::wait_on::End::External {
        session: "other".into(),
        via: "worker".into(),
    });
    live.goal = Some("review table".into());
    live.goal_set_epoch = Some(NOW - 60);
    live.branch = Some("topic".into());
    live.ae_version = Some("26.11.1".into());
    live.behind = true;
    live.created_epoch = Some(NOW - 86_400);
    live.started_epoch = Some(NOW - 3_600);
    live.last_active_epoch = Some(NOW - 59);
    let mut stopped = SessionEntry::new("old", Status::Stopped);
    stopped.origin = Some(String::new());
    stopped.unanswered = Some(0);
    let degraded = SessionEntry::degraded("unread", Status::Running);
    pin(
        "rows",
        &[stopped, live, degraded],
        include_str!("../fixtures/tableat/rows.txt"),
    );
}

#[test]
fn tableat_columns_preserve_character_padding_clipping_and_id_forms() {
    let mut s = SessionEntry::new("session-name-longer-than-26-characters", Status::Running);
    s.agents = [
        ("lead", "12345678abcdefgh"),
        ("equal", "pending"),
        ("short", "abc"),
        ("empty", ""),
        ("字界", "一二三四五六七八九"),
        ("reference-longer-than-24-characters", "12345678"),
    ]
    .into_iter()
    .map(|(name, id)| {
        let mut a = seat(name, Some("working"));
        a.alias = if name == "equal" {
            "12345678901234".into()
        } else {
            "一二三四五六七八九十甲乙丙丁戊".into()
        };
        a.session_id = Some(id.into());
        a
    })
    .collect();
    let mut legacy = seat("legacy", None);
    legacy.reference = "profile:legacy".into();
    s.agents.push(legacy);
    pin(
        "columns",
        &[s],
        include_str!("../fixtures/tableat/columns.txt"),
    );
}

#[test]
fn tableat_verdict_precedence_and_partial_knowledge_are_visible() {
    let mut exact = SessionEntry::new("exact", Status::Running);
    for (name, state, limit, unreachable) in [
        ("limit-done", Some("done"), true, true),
        ("limit-wait", Some("waiting-agent"), true, false),
        ("unreachable", Some("done"), false, true),
        ("working", Some("working"), false, false),
        ("waiting-user", Some("waiting-user"), false, false),
        ("idle", Some("idle"), false, false),
        ("absent", None, false, false),
        ("other", Some("future-state"), false, false),
    ] {
        let mut a = seat(name, state);
        a.reason = limit.then_some(Reason::Limit);
        a.unreachable = unreachable;
        a.auto_reseat_open = name == "limit-done";
        a.done_progress = Some(DoneProgress::Lapsed {
            confirmations: 1,
            required: 3,
        });
        a.declaration = Some(made("own words", Some(NOW - 120)));
        exact.agents.push(a);
    }
    let mut partial = SessionEntry::degraded("partial", Status::Running);
    partial.attention = Some(Reason::Blocked);
    partial.agents = exact.agents.clone();
    partial.agents[0].reference = "partial-limit".into();
    partial.agents[2].reference = "partial-unreachable".into();
    partial.agents.push(seat("partial-done", Some("done")));
    pin(
        "precedence",
        &[exact, partial],
        include_str!("../fixtures/tableat/precedence.txt"),
    );
}

#[test]
fn tableat_done_and_wait_proof_variants_keep_their_qualifiers() {
    let mut s = SessionEntry::new("proof", Status::Running);
    for (name, progress) in [
        ("no-record", None),
        ("none", Some(DoneProgress::None)),
        (
            "provisional",
            Some(DoneProgress::Provisional {
                confirmations: 0,
                required: 3,
            }),
        ),
        (
            "due",
            Some(DoneProgress::ChallengeDue {
                confirmations: 1,
                required: 3,
                done_age_secs: 400,
                attempts: 1,
            }),
        ),
        (
            "challenged",
            Some(DoneProgress::Challenged {
                confirmations: 2,
                required: 3,
            }),
        ),
        (
            "lapsed",
            Some(DoneProgress::Lapsed {
                confirmations: 1,
                required: 3,
            }),
        ),
        ("confirmed", Some(DoneProgress::Confirmed)),
    ] {
        let mut a = seat(name, Some("done"));
        a.done_progress = progress;
        s.agents.push(a);
    }
    for state in ["waiting-agent", "blocked"] {
        for (name, progress) in [
            ("no-record", None),
            ("none", Some(WaitProgress::None)),
            (
                "provisional",
                Some(WaitProgress::Provisional {
                    confirmations: 0,
                    required: 3,
                }),
            ),
            (
                "due",
                Some(WaitProgress::ChallengeDue {
                    confirmations: 1,
                    required: 3,
                    wait_age_secs: 400,
                    attempts: 1,
                }),
            ),
            (
                "challenged",
                Some(WaitProgress::Challenged {
                    confirmations: 2,
                    required: 3,
                }),
            ),
            (
                "lapsed",
                Some(WaitProgress::Lapsed {
                    confirmations: 1,
                    required: 3,
                }),
            ),
        ] {
            let mut a = seat(name, Some(state));
            a.wait_progress = progress;
            s.agents.push(a);
        }
    }
    pin(
        "proofs",
        &[s],
        include_str!("../fixtures/tableat/proofs.txt"),
    );
}

#[test]
fn tableat_typed_waits_and_session_wait_end_words_use_snapshot_time() {
    let mut sessions = Vec::new();
    for (index, end) in [
        crate::wait_on::End::Working,
        crate::wait_on::End::InSession,
        crate::wait_on::End::Session {
            session: "remote".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let mut s = SessionEntry::new(format!("wait-{index}"), Status::Running);
        s.wait_end = Some(end);
        for (target, overdue, missing, moved) in [
            ("peer", false, false, None),
            (
                "remote:peer",
                true,
                false,
                Some(crate::wait_on::Class::Done),
            ),
            ("@remote", false, true, None),
        ] {
            let mut a = seat("worker", Some("waiting-agent"));
            a.wait_progress = Some(WaitProgress::Lapsed {
                confirmations: 1,
                required: 3,
            });
            a.declaration = Some(Declaration::Made {
                reason: "waiting for peer evidence".into(),
                at: Some(NOW - 120),
                on: crate::wait_on::Target::parse(target, &s.name),
                overdue,
                missing,
                moved,
            });
            s.agents.push(a);
        }
        sessions.push(s);
    }
    pin(
        "waits",
        &sessions,
        include_str!("../fixtures/tableat/waits.txt"),
    );
}

#[test]
fn tableat_agent_suffix_order_and_running_dead_marker_are_preserved() {
    let mut a = seat("worker", Some("working"));
    a.observed = HarnessState::Busy;
    a.model_drift = crate::model_drift::ModelDrift::Observed("model-v2".into());
    a.harness_clash = Some(HarnessClash {
        runs: vec![crate::tool::ToolKind::Claude, crate::tool::ToolKind::Codex],
        records: "opencode\u{1b}[31m".into(),
    });
    a.effort = Some(crate::effort::Stored::Damaged(crate::effort::Damage::Bare));
    a.conversation = Some(crate::conversation::Verdict::Unknown(
        crate::conversation::Why::Unrecorded,
    ));
    a.own_work = Some(crate::session::OwnWork {
        requests: 2,
        spawns: 1,
        oldest_epoch: Some(NOW - 100),
    });
    a.declaration = Some(made("review complete", Some(NOW - 720)));
    a.alive = Some(false);
    let mut live = SessionEntry::new("live", Status::Running);
    live.attention = Some(Reason::Stale);
    live.agents = vec![a.clone()];
    live.agents[0].unreachable = true;
    let mut stopped = SessionEntry::new("stopped", Status::Stopped);
    a.observed = HarnessState::Idle;
    a.model_drift = crate::model_drift::ModelDrift::Unknown;
    stopped.agents = vec![a];
    let mut absent = seat("absent", None);
    absent.alive = Some(true);
    absent.declaration = Some(Declaration::Absent);
    let mut unknown = seat("unknown", None);
    unknown.declaration = Some(Declaration::Unreadable);
    stopped.agents.extend([absent, unknown]);
    let mut uncertain = SessionEntry::new("uncertain", Status::Unknown);
    let mut long_reason = seat("long-reason", Some("working"));
    long_reason.alive = Some(false);
    long_reason.declaration = Some(made(&"字".repeat(61), None));
    uncertain.agents.push(long_reason);
    pin(
        "suffixes",
        &[live, stopped, uncertain],
        include_str!("../fixtures/tableat/suffixes.txt"),
    );
}

#[test]
fn tableat_summary_time_edges_goal_clipping_and_blank_origins_are_pinned() {
    let mut sessions = Vec::new();
    for (index, delta) in [
        -1, 0, 59, 60, 3_599, 3_600, 86_399, 86_400, 604_799, 604_800,
    ]
    .into_iter()
    .enumerate()
    {
        let mut s = SessionEntry::new(format!("clock-{index}"), Status::Running);
        s.origin = if index % 2 == 0 {
            None
        } else {
            Some(String::new())
        };
        s.goal = Some("字".repeat(if index == 0 { 60 } else { 61 }));
        s.goal_set_epoch = Some(NOW - delta);
        s.created_epoch = Some(NOW - delta);
        s.started_epoch = Some(0);
        s.last_active_epoch = Some(-1);
        s.branch = Some(String::new());
        s.ae_version = Some(String::new());
        sessions.push(s);
    }
    pin(
        "clocks",
        &sessions,
        include_str!("../fixtures/tableat/clocks.txt"),
    );
}

#[test]
fn tableat_empty_input_and_legacy_table_clock_are_explicit() {
    assert_eq!(super::table_at(&[], Timestamp::from_epoch(NOW)), "");
    let mut s = SessionEntry::new("epoch", Status::Stopped);
    s.created_epoch = Some(1);
    let actual = super::table(&[&s]);
    assert_eq!(actual, super::table_at(&[&s], Timestamp::from_epoch(0)));
    assert_eq!(actual, include_str!("../fixtures/tableat/epoch.txt"));
}
