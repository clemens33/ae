//! O6 S1a acceptance oracle: lead rulings r1/r2 + r2 addendum a1.
//! Authored by navigator before implementation; driver installs exact bytes.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own their temporary filesystem and inspect real _run output"
)]

use super::cli::OwnedScratch;
use ae::conversation::{self as c, Cause, Probe, Start, Verdict, Why};
use ae::events::Event;
use ae::launch_cmd::Resolved;
use ae::tool::ToolKind;

const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
const ID: &str = "abcd1234-abcd-4123-8123-123456789abc";

fn key(pid: u32) -> String {
    format!("main:{LAUNCH}:{pid}")
}

fn event(action: &str, reference: &str) -> Event {
    let mut e = Event::parse_line(
        r#"{"ts":"2026-10-29T00:00:00Z","actor":"watchdog","action":"say","target":"lead","summary":"MISLEADING PROSE: resumed successfully"}"#,
    ).expect("literal journal event");
    e.action = action.into();
    e.reference = Some(reference.into());
    e
}

fn decision(action: &str, pid: u32, prior: &str, id: &str) -> Event {
    event(action, &format!("{} {prior} {id}", key(pid)))
}

fn decoded(e: &Event) -> c::Record {
    c::decode(e)
        .expect("recognized O6 action")
        .expect("valid bounded record")
}

fn verdict(events: &[Event], launched: Option<Result<&str, ()>>, live: Option<&str>) -> Verdict {
    c::current(events, true, "main", LAUNCH, launched, live)
}

#[test]
fn six_literal_actions_have_typed_facts_independent_of_summary() {
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
        let e = decision(action, 42, prior, id);
        let r = decoded(&e);
        assert_eq!(r.start, start);
        assert_eq!(r.start_key, key(42));
        assert_eq!(r.prior.as_deref(), (prior != "-").then_some(prior));
        assert_eq!(r.id, id);
        assert_eq!(c::action(start), action);
        assert_eq!(c::reference(&r), e.reference.expect("literal ref"));
    }
    for action in [
        "say",
        "memo",
        "session-id-replaced",
        "conversation-lost-future",
    ] {
        assert!(c::decode(&decision(action, 42, ID, "pending")).is_none());
    }
}

#[test]
fn hostile_ref_grammar_and_semantics_are_rejected() {
    let valid = format!("{} {ID} pending", key(42));
    let bad = [
        String::new(),
        format!(" {valid}"),
        format!("{valid} "),
        valid.replace(' ', "  "),
        valid.replace(' ', "\t"),
        format!("{valid} extra"),
        format!("{} {ID}", key(42)),
        format!("main:{LAUNCH}:0 {ID} pending"),
        format!("main:{LAUNCH}:042 {ID} pending"),
        format!("main:{LAUNCH}:+42 {ID} pending"),
        format!("main:{LAUNCH}:4294967296 {ID} pending"),
        format!("main:not-a-uuid:42 {ID} pending"),
        format!("bogus:{LAUNCH}:42 {ID} pending"),
        format!("worker.x:{LAUNCH}:42 {ID} pending"),
        format!("main:{LAUNCH}:42 ../prior pending"),
        format!("main:{LAUNCH}:42 {ID} ../fresh"),
        format!("main:{LAUNCH}:42 - pending"),
        format!("main:{LAUNCH}:42 pending pending"),
    ];
    for reference in bad {
        assert!(
            c::decode(&event("conversation-lost-not-found", &reference))
                .expect("recognized action")
                .is_err(),
            "accepted {reference:?}"
        );
    }
    for (action, prior, id) in [
        ("conversation-resumed", "-", "pending"),
        ("conversation-unproven", "-", "pending"),
        ("conversation-resumed", ID, ID),
        ("conversation-unproven", ID, ID),
        ("conversation-new", ID, "pending"),
        ("conversation-lost-no-id", ID, "pending"),
        ("conversation-lost-unreadable", "-", "pending"),
    ] {
        assert!(
            c::decode(&decision(action, 42, prior, id))
                .expect("O6")
                .is_err()
        );
    }
    let mut missing = decision("conversation-new", 42, "-", "pending");
    missing.reference = None;
    assert!(c::decode(&missing).expect("O6").is_err());
    for slot in ["main", "worker.0", "spawned.1"] {
        let reference = format!("{slot}:{LAUNCH}:42 - pending");
        assert!(
            c::decode(&event("conversation-new", &reference))
                .expect("O6")
                .is_ok()
        );
    }
}

#[test]
fn every_decision_requires_live_pid_and_optional_baseline_agreement() {
    for (action, prior, id) in [
        ("conversation-resumed", "-", ID),
        ("conversation-unproven", "-", ID),
        ("conversation-new", "-", "pending"),
        ("conversation-lost-no-id", "-", "pending"),
        ("conversation-lost-not-found", ID, "pending"),
        ("conversation-lost-unreadable", ID, "pending"),
    ] {
        let records = [decision(action, 42, prior, id)];
        let current_start = key(42);
        for baseline in [None, Some(Ok(current_start.as_str()))] {
            assert!(matches!(
                verdict(&records, baseline, Some(&key(42))),
                Verdict::Current(_)
            ));
        }
        assert!(matches!(
            verdict(&records, None, None),
            Verdict::Unknown(Why::NoLiveStart)
        ));
        assert!(matches!(
            verdict(&records, None, Some(&key(43))),
            Verdict::Unknown(Why::Stale)
        ));
        assert!(matches!(
            verdict(&records, Some(Ok(&key(43))), Some(&key(42))),
            Verdict::Unknown(Why::LaunchedMismatch)
        ));
        assert!(matches!(
            verdict(&records, Some(Err(())), Some(&key(42))),
            Verdict::Unknown(Why::LaunchedDamaged)
        ));
    }
}

#[test]
fn failed_append_failed_baseline_and_direct_rerun_never_reuse_old_truth() {
    let old = [decision("conversation-lost-not-found", 42, ID, "pending")];
    // New process 43: append failed; baseline succeeded / failed / absent.
    for baseline in [Some(key(43)), Some(key(42)), None] {
        let b = baseline.as_deref().map(Ok);
        assert!(matches!(
            verdict(&old, b, Some(&key(43))),
            Verdict::Unknown(_)
        ));
    }
    // Append succeeded, baseline failed and retained the old start.
    let new = [decision("conversation-new", 43, "-", "pending")];
    assert!(matches!(
        verdict(&new, Some(Ok(&key(42))), Some(&key(43))),
        Verdict::Unknown(Why::LaunchedMismatch)
    ));
    // Baseline absence can be proven by live pid alone; no event cannot.
    assert!(matches!(
        verdict(&new, None, Some(&key(43))),
        Verdict::Current(_)
    ));
    assert!(matches!(
        verdict(&[], Some(Ok(&key(43))), Some(&key(43))),
        Verdict::Unknown(Why::Unrecorded)
    ));
    assert!(matches!(
        verdict(&old, None, None),
        Verdict::Unknown(Why::NoLiveStart)
    ));
}

#[test]
fn journal_gaps_damage_conflict_and_latest_start_have_conservative_verdicts() {
    let old = decision("conversation-lost-not-found", 42, ID, "pending");
    let new = decision("conversation-new", 43, "-", "pending");
    let log = [old.clone(), new.clone()];
    let v = verdict(&log, Some(Ok(&key(43))), Some(&key(43)));
    let Verdict::Current(r) = v else {
        panic!("newest start must win")
    };
    assert_eq!(r.start, Start::New);
    assert!(matches!(
        c::current(&log, false, "main", LAUNCH, None, Some(&key(43))),
        Verdict::Unknown(Why::Incomplete)
    ));
    let mut damaged = new.clone();
    damaged.reference = Some(format!("{} - ../invalid", key(43)));
    for log in [
        vec![damaged.clone(), new.clone()],
        vec![new.clone(), damaged],
    ] {
        assert!(matches!(
            verdict(&log, None, Some(&key(43))),
            Verdict::Unknown(Why::Damaged)
        ));
    }
    let conflict = decision("conversation-lost-no-id", 43, "-", "pending");
    assert!(matches!(
        verdict(&[new.clone(), conflict], None, Some(&key(43))),
        Verdict::Unknown(Why::Conflict)
    ));
    assert!(matches!(
        verdict(&[new.clone(), new], None, Some(&key(43))),
        Verdict::Current(_)
    ));
    let foreign = event("conversation-new", &format!("main:{OTHER}:99 - pending"));
    assert!(matches!(
        verdict(&[old, foreign], None, Some(&key(42))),
        Verdict::Current(_)
    ));
}

#[test]
fn damage_cannot_poison_a_later_proven_start() {
    let good = decision("conversation-new", 43, "-", "pending");
    let old_damage = event("conversation-new", &format!("{} - ../bad", key(41)));
    let prefix_damage = event("conversation-new", &format!("main:{LAUNCH}:bad - pending"));
    for log in [
        vec![old_damage.clone(), good.clone()],
        vec![good.clone(), old_damage],
        vec![prefix_damage.clone(), good.clone()],
        vec![event("conversation-new", "garbage"), good.clone()],
        vec![good.clone(), event("conversation-new", "garbage")],
    ] {
        assert!(matches!(
            verdict(&log, None, Some(&key(43))),
            Verdict::Current(_)
        ));
    }
    assert!(matches!(
        verdict(&[good, prefix_damage], None, Some(&key(43))),
        Verdict::Unknown(Why::Damaged)
    ));
}

#[test]
fn probe_distinguishes_absence_read_errors_and_unprobed_stores() {
    let scratch = OwnedScratch::root("o6a", "probe");
    let home = scratch.join("codex");
    std::fs::create_dir_all(&home).expect("own home");
    let resolved = Resolved::Path(home.clone());
    for id in ["", "pending", "../bad", "has space"] {
        assert_eq!(c::probe(ToolKind::Codex, id, &resolved, None), Probe::NoId);
    }
    assert_eq!(
        c::probe(ToolKind::Codex, ID, &resolved, None),
        Probe::NotFound
    );
    let dated = home.join("sessions/2026/10/29");
    std::fs::create_dir_all(&dated).expect("dated store");
    let rollout = dated.join(format!("rollout-{ID}.jsonl"));
    std::fs::write(&rollout, "{}\n").expect("rollout");
    assert_eq!(c::probe(ToolKind::Codex, ID, &resolved, None), Probe::Found);
    std::fs::remove_file(rollout).expect("delete conversation");
    std::fs::create_dir(dated.join(format!("rollout-{ID}.jsonl"))).expect("not a rollout file");
    assert_eq!(
        c::probe(ToolKind::Codex, ID, &resolved, None),
        Probe::NotFound
    );
    std::fs::remove_dir_all(home.join("sessions")).expect("own store");
    std::fs::write(home.join("sessions"), "not a directory").expect("ENOTDIR fixture");
    assert_eq!(
        c::probe(ToolKind::Codex, ID, &resolved, None),
        Probe::Unreadable
    );
    let cwd = scratch.path();
    let transcript = home
        .join("projects")
        .join(ae::carry::project_key(cwd))
        .join(format!("{ID}.jsonl"));
    std::fs::create_dir_all(transcript).expect("directory is not a transcript");
    assert_eq!(
        c::probe(ToolKind::Claude, ID, &resolved, Some(cwd)),
        Probe::NotFound
    );
    for unresolved in [Resolved::Absent, Resolved::Unknown("$UNRESOLVED".into())] {
        assert_eq!(
            c::probe(ToolKind::Codex, ID, &unresolved, None),
            Probe::Unprobed
        );
    }
    assert_eq!(
        c::probe(ToolKind::Gemini, ID, &resolved, None),
        Probe::Unprobed
    );
}

#[test]
fn nested_probe_read_error_cannot_prove_not_found() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = OwnedScratch::root("o6a", "nested");
    let nested = scratch.join("sessions/2026");
    std::fs::create_dir_all(&nested).expect("nested store");
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o000))
        .expect("deny own dir");
    // Root can bypass permissions. Restore before any assertion or RAII cleanup.
    let denied = std::fs::read_dir(&nested).is_err();
    let result = c::probe(
        ToolKind::Codex,
        ID,
        &Resolved::Path(scratch.path().into()),
        None,
    );
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).expect("restore");
    if denied {
        assert_eq!(result, Probe::Unreadable);
    }
}

#[test]
fn lane_uses_decoded_loss_facts_as_history_and_never_trusts_prose() {
    use ae::console::lane::{self, Body, Kind, Seat};
    let seats = [Seat {
        slot: "main".into(),
        name: "lead".into(),
        profile: None,
    }];
    let records = [
        decision("conversation-lost-not-found", 42, ID, "pending"),
        decision("conversation-new", 43, "-", "pending"),
        decision("conversation-resumed", 44, "-", ID),
        decision("conversation-lost-no-id", 45, "-", "pending"),
        decision("conversation-lost-unreadable", 46, ID, "pending"),
    ];
    let lane = lane::fold(
        "fixture",
        &seats,
        &records,
        &ae::board::Observation::default(),
        0,
        &|_| Body::OldCore,
    );
    let notices: Vec<_> = lane
        .items
        .iter()
        .filter(|i| matches!(&i.kind, Kind::Said { who } if who == "ae"))
        .collect();
    assert_eq!(
        notices.len(),
        3,
        "each loss survives later starts as history"
    );
    for (notice, cause, prior) in [
        (notices[0], "not-found", "abcd1234"),
        (notices[1], "no-id", "no recorded id"),
        (notices[2], "unreadable", "abcd1234"),
    ] {
        assert_eq!(
            notice.body,
            if cause == "no-id" {
                "lead start chose a fresh conversation; no conversation id was recorded (no-id)"
                    .to_owned()
            } else {
                format!("lead start chose a fresh conversation; {prior} was not resumed ({cause})")
            }
        );
        assert!(!notice.body.contains("MISLEADING"));
    }
    let damaged = event(
        "conversation-lost-not-found",
        &format!("{} - pending", key(99)),
    );
    let lane = lane::fold(
        "fixture",
        &seats,
        &[damaged],
        &ae::board::Observation::default(),
        0,
        &|_| Body::OldCore,
    );
    assert!(
        !lane
            .items
            .iter()
            .any(|i| matches!(&i.kind, Kind::Said { who } if who == "ae"))
    );
}
