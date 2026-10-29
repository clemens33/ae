//! O6 S1b frozen oracle: brief + lead ruling r1 + plan-a1 public seam.
//! External observations are inputs; expected decisions are literal requirements.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private fixtures and supplies independent process/journal facts"
)]

use std::cell::Cell;

use ae::conversation::{Record, Start, Verdict, Why};
use ae::digest::{AgentEntry, SessionEntry, Status};
use ae::events::{Cursor, Drain, Event, EventError, SkippedLine};
use ae::json::Value;
use ae::meta::Meta;
use ae::procs::Proc;
use ae::session::{SessionRead, conversation_of};

const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
const ID: &str = "abcd1234-abcd-4123-8123-123456789abc";

fn key(pid: u32) -> String {
    format!("main:{LAUNCH}:{pid}")
}

fn meta(extra: &str) -> Meta {
    Meta::parse(&format!(
        "schema=2\nseat.main=lead\nprofile.main=fake\nagent_bin.main=codex\nlaunch_id.main={LAUNCH}\n{extra}"
    ))
}

fn baseline(pid: u32) -> String {
    format!(
        "launched.main={}\nlaunched_model.main=-\nlaunched_effort.main=-\n",
        key(pid)
    )
}

fn event(action: &str, reference: &str) -> Event {
    Event::parse_line(&format!(
        "{{\"ts\":\"2026-10-29T00:00:00Z\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"lead\",\"ref\":\"{reference}\",\"summary\":\"MISLEADING: resumed\"}}"
    )).expect("independent literal journal event")
}

fn decision(action: &str, pid: u32, prior: &str, id: &str) -> Event {
    event(action, &format!("{} {prior} {id}", key(pid)))
}

fn read(events: Vec<Event>, complete: bool) -> SessionRead {
    SessionRead::from_drain(&Drain {
        events,
        cursor: Cursor::default(),
        skipped: if complete {
            vec![]
        } else {
            vec![SkippedLine {
                generation: 0,
                offset: 1,
                reason: EventError::MissingKey("ts"),
            }]
        },
        drained: true,
    })
}

fn table() -> Vec<Proc> {
    vec![
        Proc {
            pid: 100,
            ppid: 1,
            comm: "sh".into(),
        },
        Proc {
            pid: 420,
            ppid: 100,
            comm: "codex".into(),
        },
        // A nested same-kind subprocess must not replace the outer tool start.
        Proc {
            pid: 421,
            ppid: 420,
            comm: "codex".into(),
        },
    ]
}

fn consume(meta: &Meta, read: Option<&SessionRead>, procs: Option<&[Proc]>) -> Option<Verdict> {
    conversation_of(
        meta,
        read,
        "main",
        Status::Running,
        Some(true),
        Some(100),
        &|| procs,
    )
}

fn unknown(v: Option<Verdict>, why: Why) {
    assert_eq!(v.expect("computed verdict"), Verdict::Unknown(why));
}

fn rendered(agent: AgentEntry) -> String {
    let mut session = SessionEntry::new("fixture", Status::Running);
    session.agents.push(agent);
    let world = ae::listing::World::new(
        ae::time::Timestamp::from_epoch(1_792_000_000),
        vec![session],
    );
    ae::listing::render(
        &ae::filters::ListArgs::parse::<&str>(&[]).expect("default list"),
        &world,
    )
}

#[test]
fn six_choices_flow_through_the_consumer_not_summary_or_pane_root_pid() {
    use ae::conversation::Cause;
    for (action, start, prior, id) in [
        ("conversation-resumed", Start::Resumed, "-", ID),
        ("conversation-unproven", Start::Unproven, "-", ID),
        ("conversation-new", Start::New, "-", "pending"),
        (
            "conversation-lost-no-id",
            Start::Lost(Cause::NoId),
            "-",
            "pending",
        ),
        (
            "conversation-lost-not-found",
            Start::Lost(Cause::NotFound),
            ID,
            "pending",
        ),
        (
            "conversation-lost-unreadable",
            Start::Lost(Cause::Unreadable),
            ID,
            "fresh-42",
        ),
    ] {
        let r = read(vec![decision(action, 420, prior, id)], true);
        for rows in [String::new(), baseline(420)] {
            assert_eq!(
                consume(&meta(&rows), Some(&r), Some(&table())),
                Some(Verdict::Current(Record {
                    start,
                    start_key: key(420),
                    prior: (prior != "-").then(|| prior.into()),
                    id: id.into(),
                })),
                "{action}; baseline={rows:?}"
            );
        }
        unknown(consume(&meta(""), Some(&r), None), Why::NoLiveStart);
        unknown(
            consume(&meta(&baseline(419)), Some(&r), Some(&table())),
            Why::LaunchedMismatch,
        );
        unknown(
            consume(&meta("launched.main=broken\n"), Some(&r), Some(&table())),
            Why::LaunchedDamaged,
        );
    }
}

#[test]
fn journal_or_identity_loss_is_unknown_and_precedes_live_proof() {
    let good = decision("conversation-new", 420, "-", "pending");
    let incomplete = read(vec![good.clone()], false);
    unknown(
        consume(&meta(""), Some(&incomplete), Some(&table())),
        Why::Incomplete,
    );
    unknown(consume(&meta(""), None, Some(&table())), Why::Incomplete);
    let missing = Meta::parse("schema=2\nseat.main=lead\nagent_bin.main=codex\n");
    let doubled = meta(&format!("launch_id.main={LAUNCH}\n"));
    for m in [missing, doubled] {
        unknown(
            consume(&m, Some(&read(vec![good.clone()], true)), Some(&table())),
            Why::Incomplete,
        );
    }
    let r = read(vec![good], true);
    let procs = table();
    for (status, alive, pid) in [
        (Status::Unknown, Some(true), Some(100)),
        (Status::Running, None, Some(100)),
        (Status::Running, Some(true), None),
    ] {
        unknown(
            conversation_of(&meta(""), Some(&r), "main", status, alive, pid, &|| {
                Some(procs.as_slice())
            }),
            Why::NoLiveStart,
        );
    }
    let wrong_tool = Meta::parse(&format!(
        "schema=2\nseat.main=lead\nagent_bin.main=unknown\nlaunch_id.main={LAUNCH}\n"
    ));
    unknown(
        consume(&wrong_tool, Some(&r), Some(&table())),
        Why::NoLiveStart,
    );
    unknown(consume(&meta(""), Some(&r), Some(&[])), Why::NoLiveStart);
    let mut ambiguous = table();
    ambiguous.push(Proc {
        pid: 430,
        ppid: 100,
        comm: "codex".into(),
    });
    unknown(
        consume(&meta(""), Some(&r), Some(&ambiguous)),
        Why::NoLiveStart,
    );
    for (status, alive) in [
        (Status::Stopped, Some(true)),
        (Status::Running, Some(false)),
    ] {
        assert_eq!(
            conversation_of(
                &meta(""),
                None,
                "main",
                status,
                alive,
                Some(100),
                &|| panic!("non-applicable cannot read ps")
            ),
            None
        );
    }
}

#[test]
fn damage_scope_order_conflict_and_failed_write_equivalents_use_live_start() {
    let good = decision("conversation-new", 420, "-", "pending");
    let damaged = event("conversation-new", &format!("{} - ../bad", key(420)));
    let foreign_damage = event("conversation-new", &format!("{} - ../bad", key(419)));
    let prefix_damage = event("conversation-new", &format!("main:{LAUNCH}:bad - pending"));
    for events in [
        vec![damaged.clone()],
        vec![good.clone(), damaged.clone()],
        vec![damaged, good.clone()],
        vec![good.clone(), prefix_damage.clone()],
    ] {
        unknown(
            consume(&meta(""), Some(&read(events, true)), Some(&table())),
            Why::Damaged,
        );
    }
    unknown(
        consume(
            &meta(""),
            Some(&read(vec![foreign_damage.clone()], true)),
            Some(&table()),
        ),
        Why::Unrecorded,
    );
    for events in [
        vec![foreign_damage.clone(), good.clone()],
        vec![good.clone(), foreign_damage],
        vec![prefix_damage, good.clone()],
        vec![good.clone(), good.clone()],
    ] {
        assert!(matches!(
            consume(&meta(""), Some(&read(events, true)), Some(&table())),
            Some(Verdict::Current(_))
        ));
    }
    unknown(
        consume(
            &meta(""),
            Some(&read(
                vec![
                    good.clone(),
                    decision("conversation-lost-no-id", 420, "-", "pending"),
                ],
                true,
            )),
            Some(&table()),
        ),
        Why::Conflict,
    );
    // Append failed; baseline succeeded, failed retaining old, or was absent. Direct rerun/dead-old-pid too.
    let old = read(
        vec![decision("conversation-lost-not-found", 419, ID, "pending")],
        true,
    );
    for rows in [baseline(420), baseline(419), String::new()] {
        unknown(
            consume(&meta(&rows), Some(&old), Some(&table())),
            Why::Stale,
        );
    }
    let current = read(vec![good], true);
    unknown(
        consume(&meta(&baseline(419)), Some(&current), Some(&table())),
        Why::LaunchedMismatch,
    );
    unknown(
        consume(
            &meta(""),
            Some(&read(
                vec![event(
                    "conversation-new",
                    &format!("main:{OTHER}:420 - pending"),
                )],
                true,
            )),
            Some(&table()),
        ),
        Why::Unrecorded,
    );
}

#[test]
fn lazy_snapshot_includes_damaged_records_but_skips_unrelated_history() {
    let procs = table();
    for (events, expected_calls) in [
        (vec![], 0),
        (vec![event("say", "ordinary")], 0),
        (
            vec![event(
                "conversation-new",
                &format!("main:{OTHER}:420 - pending"),
            )],
            0,
        ),
        (vec![decision("conversation-new", 420, "-", "pending")], 1),
        (
            vec![event("conversation-new", &format!("{} - ../bad", key(420)))],
            1,
        ),
        (
            vec![event("conversation-new", &format!("{} - ../bad", key(419)))],
            1,
        ),
        (
            vec![event(
                "conversation-new",
                &format!("main:{LAUNCH}:bad - pending"),
            )],
            1,
        ),
    ] {
        let calls = Cell::new(0);
        let get = || {
            calls.set(calls.get() + 1);
            Some(procs.as_slice())
        };
        let _ = conversation_of(
            &meta(""),
            Some(&read(events, true)),
            "main",
            Status::Running,
            Some(true),
            Some(100),
            &get,
        );
        assert_eq!(calls.get(), expected_calls);
    }
}

#[test]
fn format_and_pid_parsing_preserve_the_row_and_refuse_bad_arity() {
    assert!(ae::tmux::ALL_PANES_FORMAT.ends_with(" | #{pane_id} | #{pane_pid}"));
    for (raw, pid) in [
        ("1234", Some(1234)),
        ("", None),
        ("0", None),
        ("+5", None),
        ("-1", None),
        ("01", None),
        ("4294967296", None),
        ("x", None),
    ] {
        let all = ae::tmux::interpret_all_panes(
            true,
            &format!("fixture | branch | 0 | main | idle | codex | %1 | {raw}"),
        )
        .expect("pane enumeration");
        let rows = all
            .session("fixture")
            .expect("bad pid never discards known session");
        assert_eq!(rows.panes.len(), 1);
        assert_eq!(rows.panes[0].pid, pid);
        assert_eq!(rows.panes[0].slot.as_deref(), Some("main"));
    }
    for raw in [
        "fixture | branch | 0 | main | idle | codex | %1",
        "fixture | branch | 0 | main | idle | codex | %1 | 100 | extra",
    ] {
        assert!(
            ae::tmux::interpret_all_panes(true, raw)
                .expect("enumeration")
                .session("fixture")
                .is_none()
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn verdict_cells_and_json_have_one_truth_without_changing_session_id() {
    use ae::conversation::Cause;
    for (start, cell, word, cause, prior) in [
        (Start::Resumed, None, "resumed", None, None),
        (
            Start::Unproven,
            Some(" · conv:unproven"),
            "unproven",
            None,
            None,
        ),
        (Start::New, Some(" · conv:new"), "new", None, None),
        (
            Start::Lost(Cause::NoId),
            Some(" · conv:lost(no-id)"),
            "lost",
            Some("no-id"),
            None,
        ),
        (
            Start::Lost(Cause::NotFound),
            Some(" · conv:lost(not-found)"),
            "lost",
            Some("not-found"),
            Some(ID),
        ),
        (
            Start::Lost(Cause::Unreadable),
            Some(" · conv:lost(unreadable)"),
            "lost",
            Some("unreadable"),
            Some(ID),
        ),
    ] {
        let v = Verdict::Current(Record {
            start,
            start_key: key(420),
            prior: prior.map(str::to_owned),
            id: ID.into(),
        });
        assert_eq!(v.cell().as_deref(), cell);
        assert_eq!(v.why(), None);
        let agent = AgentEntry {
            conversation: Some(v),
            session_id: Some("pending".into()),
            ..AgentEntry::default()
        };
        let json = agent.to_json();
        let text = rendered(agent);
        assert_eq!(text.contains(" · conv:"), cell.is_some(), "{text}");
        if let Some(cell) = cell {
            assert!(text.contains(cell), "{text}");
        }
        assert_eq!(json.get_str("session_id"), Some("-"));
        let c = json.get("conversation").expect("additive object");
        for (field, want) in [
            ("start", Some(word)),
            ("cause", cause),
            ("prior", prior),
            ("current", Some("yes")),
            ("why", None),
        ] {
            assert_eq!(
                c.get(field),
                Some(&want.map_or(Value::Null, |s| Value::Str(s.into()))),
                "{field}: {c}"
            );
        }
    }
    for (why, word) in [
        (Why::Unrecorded, "unrecorded"),
        (Why::Incomplete, "incomplete"),
        (Why::Damaged, "damaged"),
        (Why::NoLiveStart, "no-live-start"),
        (Why::Stale, "stale"),
        (Why::LaunchedMismatch, "launched-mismatch"),
        (Why::LaunchedDamaged, "launched-damaged"),
        (Why::Conflict, "conflict"),
    ] {
        let v = Verdict::Unknown(why);
        assert_eq!(
            v.cell().as_deref(),
            Some(if why == Why::Unrecorded {
                " · conv:unrecorded"
            } else {
                " · conv:unknown"
            })
        );
        assert_eq!(v.why(), Some(word));
        let json = AgentEntry {
            conversation: Some(v),
            ..AgentEntry::default()
        }
        .to_json();
        let c = json
            .get("conversation")
            .expect("unknown is object, not null");
        assert_eq!(c.get_str("current"), Some("unknown"));
        assert_eq!(c.get_str("why"), Some(word));
        for field in ["start", "cause", "prior"] {
            assert_eq!(c.get(field), Some(&Value::Null));
        }
    }
    assert_eq!(
        AgentEntry::default().to_json().get("conversation"),
        Some(&Value::Null)
    );
    let text = rendered(AgentEntry {
        name: "lead".into(),
        reference: "lead".into(),
        session_id: Some("pending".into()),
        ..AgentEntry::default()
    });
    assert!(
        text.contains("pending"),
        "table-only literal pending: {text}"
    );
}
