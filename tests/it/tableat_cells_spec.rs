//! The seams `table_at` is assembled from: the session line, the verdict
//! cell, the `done` proof cell, the drift cell and the agent line.
//!
//! Oracle: the cell words and their order are the `ae list` section of
//! docs/reference/commands.md; column widths are the table's fixed 26/24/14/10
//! (a behaviour pin, as in the frozen `tableat_spec`). Expected lines are built
//! here from those widths and words, never by calling the unit under test.

use super::{done_cell, push_agent_line, push_drift, push_session_line, push_verdict};
use crate::attention::Reason;
use crate::digest::{AgentEntry, SessionEntry, Status};
use crate::harness_state::HarnessState;
use crate::model_drift::ModelDrift;
use crate::time::Timestamp;
use crate::watchdog::{DoneProgress, WaitProgress};

const NOW: Timestamp = Timestamp::from_epoch(2_000_000);

fn seat(state: Option<&str>) -> AgentEntry {
    AgentEntry {
        reference: "lead".into(),
        name: "lead".into(),
        alias: "profile".into(),
        state: state.map(str::to_owned),
        ..AgentEntry::default()
    }
}

fn verdict(session: &SessionEntry, agent: &AgentEntry) -> String {
    let mut out = String::new();
    push_verdict(&mut out, session, agent, NOW);
    out
}

fn session_line(session: &SessionEntry) -> String {
    let mut out = String::new();
    push_session_line(&mut out, session);
    out
}

#[test]
fn a_session_line_without_an_origin_ends_at_its_last_word() {
    let running = SessionEntry::new("s", Status::Running);
    assert_eq!(session_line(&running), format!("{:<26} running\n", "s"));
    let mut blank = SessionEntry::new("s", Status::Stopped);
    blank.origin = Some(String::new());
    assert_eq!(session_line(&blank), format!("{:<26} stopped\n", "s"));
}

#[test]
fn a_session_line_keeps_the_origin_bytes_and_puts_markers_before_it() {
    let mut s = SessionEntry::new("s", Status::Running);
    s.origin = Some("/work/s ".into());
    assert_eq!(
        session_line(&s),
        format!("{:<26} {:<10} /work/s \n", "s", "running")
    );
    s.attention = Some(Reason::Blocked);
    s.unanswered = Some(2);
    assert_eq!(
        session_line(&s),
        format!(
            "{:<26} {:<10} attn:blocked unanswered:2 /work/s \n",
            "s", "running"
        )
    );
}

#[test]
fn a_session_line_appends_to_the_output_and_trims_only_its_own_trailing_spaces() {
    let mut out = String::from("earlier line \n");
    push_session_line(&mut out, &SessionEntry::new("s", Status::Running));
    assert_eq!(out, format!("earlier line \n{:<26} running\n", "s"));
}

#[test]
fn a_done_seat_names_its_proof_episode_only_while_one_runs() {
    let cell = |progress| {
        let mut out = String::new();
        done_cell(progress, &mut out);
        out
    };
    for (progress, words) in [
        (
            DoneProgress::Provisional {
                confirmations: 0,
                required: 2,
            },
            "finished (unconfirmed 0/2)",
        ),
        (
            DoneProgress::ChallengeDue {
                confirmations: 1,
                required: 2,
                done_age_secs: 900,
                attempts: 3,
            },
            "finished (unconfirmed 1/2)",
        ),
        (
            DoneProgress::Challenged {
                confirmations: 1,
                required: 3,
            },
            "finished (unconfirmed 1/3)",
        ),
        (
            DoneProgress::Lapsed {
                confirmations: 2,
                required: 3,
            },
            "finished (unconfirmed 2/3, lapsed)",
        ),
        (DoneProgress::Confirmed, "finished"),
        (DoneProgress::None, "finished"),
    ] {
        assert_eq!(cell(Some(progress)), words, "{progress:?}");
    }
    assert_eq!(cell(None), "finished");
}

#[test]
fn a_usage_limit_outranks_every_other_verdict_and_names_an_open_reseat() {
    let exact = SessionEntry::new("s", Status::Running);
    let mut a = seat(Some("done"));
    a.reason = Some(Reason::Limit);
    a.unreachable = true;
    assert_eq!(verdict(&exact, &a), "needs you (usage limit)");
    a.auto_reseat_open = true;
    assert_eq!(
        verdict(&exact, &a),
        "needs you (usage limit) · auto-reseat in flight"
    );
    // The limit is a pane fact: an untrusted event source does not hide it.
    let partial = SessionEntry::degraded("s", Status::Running);
    assert_eq!(
        verdict(&partial, &a),
        "needs you (usage limit) · auto-reseat in flight"
    );
}

#[test]
fn an_unreachable_seat_outranks_its_declared_state() {
    let exact = SessionEntry::new("s", Status::Running);
    for state in [Some("done"), Some("blocked"), Some("working"), None] {
        let mut a = seat(state);
        a.unreachable = true;
        assert_eq!(
            verdict(&exact, &a),
            "unreachable (no delivery)",
            "{state:?}"
        );
    }
}

#[test]
fn a_done_seat_reads_finished_with_its_proof_and_unknown_when_the_source_is_partial() {
    let exact = SessionEntry::new("s", Status::Running);
    let mut a = seat(Some("done"));
    assert_eq!(verdict(&exact, &a), "finished");
    a.done_progress = Some(DoneProgress::Lapsed {
        confirmations: 1,
        required: 3,
    });
    assert_eq!(verdict(&exact, &a), "finished (unconfirmed 1/3, lapsed)");
    let partial = SessionEntry::degraded("s", Status::Running);
    assert_eq!(verdict(&partial, &a), "unknown");
}

#[test]
fn a_waiting_seat_reads_its_task_word_with_its_proof_and_unknown_when_partial() {
    let exact = SessionEntry::new("s", Status::Running);
    let partial = SessionEntry::degraded("s", Status::Running);
    for (state, words) in [
        ("waiting-agent", "waiting on an agent"),
        ("blocked", "needs you (blocked)"),
    ] {
        let mut a = seat(Some(state));
        assert_eq!(verdict(&exact, &a), words, "{state}");
        a.wait_progress = Some(WaitProgress::Challenged {
            confirmations: 2,
            required: 3,
        });
        assert_eq!(
            verdict(&exact, &a),
            format!("{words} (unconfirmed 2/3)"),
            "{state}"
        );
        assert_eq!(verdict(&partial, &a), "unknown", "{state}");
    }
}

#[test]
fn any_other_state_reads_its_task_word_and_a_missing_one_reads_no_current_status() {
    let exact = SessionEntry::new("s", Status::Running);
    let partial = SessionEntry::degraded("s", Status::Running);
    for (state, words) in [
        (Some("working"), "working"),
        (Some("waiting-user"), "needs you (waiting on you)"),
        (Some("idle"), "no current status"),
        (Some("future-state"), "future-state"),
        (None, "no current status"),
    ] {
        assert_eq!(verdict(&exact, &seat(state)), words, "{state:?}");
        assert_eq!(verdict(&partial, &seat(state)), "unknown", "{state:?}");
    }
}

#[test]
fn model_drift_adds_nothing_when_quiet_and_says_so_loudly_otherwise() {
    let cell = |drift: ModelDrift| {
        let mut out = String::from("x");
        push_drift(&mut out, &drift);
        out
    };
    assert_eq!(cell(ModelDrift::Quiet), "x");
    assert_eq!(cell(ModelDrift::Observed("m-2".into())), "x · drift:m-2");
    assert_eq!(cell(ModelDrift::Unknown), "x · drift:unknown");
}

#[test]
fn an_agent_line_lays_out_its_columns_and_ends_with_the_observation() {
    let session = SessionEntry::new("s", Status::Running);
    let mut a = seat(Some("working"));
    a.alias = "claude-profile".into();
    a.session_id = Some("abcdef0123456789".into());
    a.observed = HarnessState::Idle;
    let mut out = String::new();
    push_agent_line(&mut out, &session, &a, NOW);
    assert_eq!(
        out,
        format!(
            "  {:<24} {:<14} {:<10} working · observed:input ready\n",
            "lead", "claude-profile", "abcdef01"
        )
    );
}

#[test]
fn an_agent_line_blanks_the_profile_of_a_legacy_reference() {
    let session = SessionEntry::new("s", Status::Running);
    let mut a = seat(Some("working"));
    a.reference = "profile:lead".into();
    let mut out = String::new();
    push_agent_line(&mut out, &session, &a, NOW);
    assert_eq!(
        out,
        format!(
            "  {:<24} {:<14} {:<10} working · observed:unknown\n",
            "profile:lead", "", "-"
        )
    );
}

#[test]
fn the_dead_marker_needs_a_running_session_and_a_proven_dead_pane() {
    let line = |status, alive| {
        let session = SessionEntry::new("s", status);
        let mut a = seat(Some("working"));
        a.alive = alive;
        let mut out = String::new();
        push_agent_line(&mut out, &session, &a, NOW);
        out
    };
    assert!(line(Status::Running, Some(false)).ends_with(" · observed:unknown !\n"));
    for (status, alive) in [
        (Status::Running, Some(true)),
        (Status::Running, None),
        (Status::Stopped, Some(false)),
        (Status::Unknown, Some(false)),
    ] {
        assert!(
            line(status, alive).ends_with(" · observed:unknown\n"),
            "{status:?} {alive:?}"
        );
    }
}
