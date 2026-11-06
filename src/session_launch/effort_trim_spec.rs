//! #308 trim-orphan acceptance, independently authored by effort308-nav.
//! Oracle: a retained effort record must keep its journal authority through
//! resume retention. A trim must never invent authority by erasing a yield,
//! discard, or damaged journal evidence. No live vendor or tmux process runs.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance fixtures own and inspect scratch meta and journals"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::effort::{
    Decision, Direction, Episode, Fold, Inputs, Kept, Leg, Level, Record, Resume, Scope, Stored,
    Switch, Transition, TwinRefusal, Window, decide, fold, on_resume,
};
use crate::events::{Event, SessionKey};
use crate::quota::WindowKey;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const EP: i64 = 1_700_000_000;
const VALID: &str = "v1 applied slot:main base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:tok-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:custom client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700001000";

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ae-effort308-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).expect("exclusively owned fixture directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn event(action: &str, reference: &str, summary: &str) -> String {
    format!(
        "{{\"ts\":\"2023-11-14T22:13:20Z\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"lead\",\"target_slot\":\"main\",\"target_session\":\"fixture\",\"target_session_id\":\"{SID}\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}\n"
    )
}

fn applied() -> Vec<String> {
    vec![
        event("effort-episode", "1700000000", "threshold 80"),
        event("effort-reserved", "1700000000-down", "xhigh->high"),
        event("effort-applied", "1700000000-down", "xhigh->high"),
    ]
}

fn padding(count: usize) -> String {
    let mut text = String::new();
    for n in 0..count {
        writeln!(
            text,
            "{{\"ts\":\"2023-11-14T22:13:30Z\",\"actor\":\"human\",\"action\":\"memo\",\"summary\":\"ordinary-{n}\"}}"
        )
        .expect("writing a fixture into a String");
    }
    text
}

fn meta(record: &str) -> String {
    format!(
        "schema=2\nmeta_version=2\nsession=fixture\nsession_id={SID}\nseat.main=lead\nprofile.main=custom\nagent_bin.main=codex\nlaunch_id.main=tok-1\nharness_session.main=22222222-2222-4222-8222-222222222222\neffort.main={record}\n"
    )
}

fn folded(journal: &str) -> Fold {
    folded_for(journal, "main", "lead")
}

fn folded_for(journal: &str, slot: &str, agent: &str) -> Fold {
    let events: Vec<_> = journal
        .lines()
        .map(|line| Event::parse_line(line).expect("independently authored event fixture"))
        .collect();
    fold(&events, SessionKey::new("fixture", SID), slot, agent)
}

fn expected_applied(yielded: bool) -> Fold {
    Fold::Open(Episode {
        key: EP,
        threshold: 80,
        down: Leg::Done(Transition {
            direction: Direction::Down,
            from: Level::Xhigh,
            to: Level::High,
        }),
        up: Leg::Unspent,
        named: vec![],
        yielded,
    })
}

fn retained(tag: &str, record: &str, journal: &str) -> (String, Stored) {
    let bytes = meta(record);
    let first = retained_document(tag, &bytes, journal);
    let stored = crate::effort::stored(bytes.as_bytes(), "main");
    assert!(matches!(stored, Stored::Present(_)), "valid retained claim");
    (first, stored)
}

fn retained_document(tag: &str, bytes: &str, journal: &str) -> String {
    let dir = Scratch::new(tag);
    let path = dir.path().join("events.jsonl");
    std::fs::write(dir.path().join("meta"), bytes).expect("retained seat meta");
    std::fs::write(&path, journal).expect("journal before resume");
    assert!(
        journal.lines().count() > 1000,
        "the production trim must run"
    );

    super::trim_events(dir.path());

    assert_eq!(
        std::fs::read_to_string(dir.path().join("meta")).expect("meta after trim"),
        bytes,
        "retention must not clear the applied record to hide lost authority"
    );
    let first = std::fs::read_to_string(&path).expect("journal after resume trim");
    super::trim_events(dir.path());
    assert_eq!(
        std::fs::read_to_string(path).expect("second resume trim"),
        first,
        "retention must be idempotent"
    );
    first
}

fn replay(stored: &Stored, ledger: &Fold) -> Resume {
    on_resume(stored, Ok(()), Some(ledger), Switch::On, true)
}

#[test]
fn applied_episode_older_than_the_resume_window_keeps_its_replay_authority() {
    let journal = applied().concat() + &padding(1100);
    assert_eq!(folded(&journal), expected_applied(false));
    let (kept, stored) = retained("aged", VALID, &journal);
    let ledger = folded(&kept);
    assert_eq!(ledger, expected_applied(false));
    assert_eq!(replay(&stored, &ledger), Resume::Replay(Level::High));
    assert!(
        !kept.contains("ordinary-0\""),
        "ordinary history still ages out"
    );
    assert!(
        kept.contains("ordinary-1099\""),
        "the newest window remains"
    );
}

#[test]
fn a_resume_cut_inside_the_applied_episode_keeps_the_dropped_opening() {
    let trace = applied();
    let journal = trace[0].clone() + &padding(200) + &trace[1] + &trace[2] + &padding(998);
    assert_eq!(folded(&journal), expected_applied(false));
    let (kept, stored) = retained("split", VALID, &journal);
    assert_eq!(folded(&kept), expected_applied(false));
    assert_eq!(replay(&stored, &folded(&kept)), Resume::Replay(Level::High));
}

#[test]
fn recovery_after_resume_still_requests_the_step_back_up() {
    let journal = applied().concat() + &padding(1100);
    let (kept, stored) = retained("restore", VALID, &journal);
    let ledger = folded(&kept);
    let record = Record::parse(VALID).expect("literal applied record");
    let no_twin = Err(TwinRefusal::NoTwin(Level::High));
    // Contract pin: docs/reference/contract.md auto-effort invariant, threshold
    // 80 minus src/autoreseat.rs:183 REARM_GAP = 5, strictly below 75 to recover.
    for (percent, expected) in [
        (74.0, Decision::Up),
        (75.0, Decision::Hold(crate::effort::Hold::Pressure)),
    ] {
        let scope = Scope {
            windows: vec![Window {
                key: WindowKey {
                    bucket: "weekly".into(),
                    qualifier: None,
                    minutes: Some(10_080),
                },
                judged: percent,
                usable: true,
                observed_at: EP + 10,
                resets_at: Some(EP + 1000),
            }],
            spend_capped: false,
        };
        assert_eq!(
            decide(&Inputs {
                at: 80,
                new_attempts: true,
                scope: Some(&scope),
                limited: false,
                fold: &ledger,
                stored: &stored,
                guard: Some(Ok(())),
                twin: &no_twin,
                live: Some(record.low),
                now: EP + 10,
            }),
            expected,
            "the retained history must support the unchanged recovery band"
        );
    }
}

#[test]
fn a_yield_that_aged_out_with_the_episode_still_blocks_replay() {
    let journal = applied().concat()
        + &event("effort-yielded", "1700000000", "manual-effort")
        + &padding(1100);
    assert_eq!(folded(&journal), expected_applied(true));
    let (kept, stored) = retained("yielded", VALID, &journal);
    let ledger = folded(&kept);
    assert_eq!(ledger, expected_applied(true));
    assert_eq!(replay(&stored, &ledger), Resume::Keep(Kept::Yielded));
}

#[test]
fn a_journaled_discard_with_a_failed_meta_clear_never_regains_authority() {
    // discard_effort appends first. A later failed publication leaves this
    // product-valid combination: the discard is durable and the old meta stands.
    let journal =
        applied().concat() + &event("effort-discarded", "1700000000", "off") + &padding(1100);
    assert_eq!(folded(&journal), Fold::Armed);
    let (kept, stored) = retained("discarded", VALID, &journal);
    let ledger = folded(&kept);
    assert_eq!(ledger, Fold::Armed);
    assert_eq!(replay(&stored, &ledger), Resume::Keep(Kept::Ledger));
}

#[test]
fn duplicate_openings_do_not_become_a_valid_episode_after_retention() {
    let journal = event("effort-episode", "1700000000", "threshold 80")
        + &applied().concat()
        + &padding(1100);
    let expected = Fold::Doubt("effort episode opened out of order");
    assert_eq!(folded(&journal), expected);
    let (kept, stored) = retained("duplicate", VALID, &journal);
    let ledger = folded(&kept);
    assert_eq!(ledger, expected);
    assert_eq!(replay(&stored, &ledger), Resume::Keep(Kept::Ledger));
}

#[test]
fn malformed_later_effort_evidence_is_not_erased_by_an_episode_ref_filter() {
    for (reference, reason) in [
        ("not-an-epoch", "unreadable effort ref"),
        ("1700000001", "effort record names no open episode"),
    ] {
        let journal =
            applied().concat() + &event("effort-held", reference, "unreadable") + &padding(1100);
        let expected = Fold::Doubt(reason);
        assert_eq!(folded(&journal), expected);
        let (kept, stored) = retained("malformed", VALID, &journal);
        let ledger = folded(&kept);
        assert_eq!(ledger, expected);
        assert_eq!(replay(&stored, &ledger), Resume::Keep(Kept::Ledger));
    }
}

fn closed_history(restored: bool) -> String {
    let ending = if restored {
        event("effort-reserved", "1700000000-up", "high->xhigh")
            + &event("effort-restored", "1700000000-up", "high->xhigh")
    } else {
        event("effort-discarded", "1700000000", "off")
    };
    (applied().concat() + &ending)
        .replace("1700000000", "1699999900")
        .replace("22:13:20Z", "22:11:40Z")
}

fn newest_window(journal: &str) -> String {
    let lines: Vec<_> = journal.split_inclusive('\n').collect();
    lines[lines.len() - 1000..].concat()
}

#[test]
fn a_closed_episode_without_retained_meta_ages_out_completely() {
    let document = meta(VALID).replace(&format!("effort.main={VALID}\n"), "");
    for restored in [false, true] {
        let journal = closed_history(restored) + &padding(1100);
        assert_eq!(folded(&journal), Fold::Armed, "the old episode is closed");
        let kept = retained_document("closed", &document, &journal);
        assert_eq!(kept, newest_window(&journal));
        assert_eq!(kept.lines().count(), 1000);
        assert!(
            !kept.contains("effort-"),
            "closed effort history must expire"
        );
    }
}

#[test]
fn only_the_active_episode_adds_evidence_to_the_newest_window() {
    let active = applied().concat();
    let journal = closed_history(true) + &active + &padding(1100);
    assert_eq!(folded(&journal), expected_applied(false));
    let (kept, stored) = retained("exact", VALID, &journal);
    assert_eq!(
        kept,
        active + &newest_window(&journal),
        "the trim retains exactly the three active records plus the newest 1000"
    );
    assert_eq!(kept.lines().count(), 1003);
    assert_eq!(replay(&stored, &folded(&kept)), Resume::Replay(Level::High));
}

fn worker_record() -> String {
    VALID
        .replace("slot:main", "slot:worker.0")
        .replace("agent:lead", "agent:colead")
        .replace("ep:1700000000", "ep:1700000001")
        .replace("launch:tok-1 pid:42", "launch:tok-2 pid:43")
        .replace(
            "conv:22222222-2222-4222-8222-222222222222",
            "conv:33333333-3333-4333-8333-333333333333",
        )
}

fn worker_document() -> String {
    format!(
        "seat.worker.0=colead\nprofile.worker.0=custom\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-2\nharness_session.worker.0=33333333-3333-4333-8333-333333333333\neffort.worker.0={}\n",
        worker_record()
    )
}

fn worker_history() -> String {
    applied()
        .concat()
        .replace("\"target_slot\":\"main\"", "\"target_slot\":\"worker.0\"")
        .replace("\"target\":\"lead\"", "\"target\":\"colead\"")
        .replace("1700000000", "1700000001")
        .replace("22:13:20Z", "22:13:21Z")
}

fn expected_worker() -> Fold {
    Fold::Open(Episode {
        key: EP + 1,
        threshold: 80,
        down: Leg::Done(Transition {
            direction: Direction::Down,
            from: Level::Xhigh,
            to: Level::High,
        }),
        up: Leg::Unspent,
        named: vec![],
        yielded: false,
    })
}

#[test]
fn both_lead_pair_slots_keep_their_own_applied_episode() {
    let document = meta(VALID) + &worker_document();
    let active = applied().concat() + &worker_history();
    let journal = active.clone() + &padding(1100);
    assert_eq!(folded(&journal), expected_applied(false));
    assert_eq!(
        folded_for(&journal, "worker.0", "colead"),
        expected_worker()
    );
    let kept = retained_document("pair", &document, &journal);
    assert_eq!(kept, active + &newest_window(&journal));
    for (slot, agent, expected) in [
        ("main", "lead", expected_applied(false)),
        ("worker.0", "colead", expected_worker()),
    ] {
        let ledger = folded_for(&kept, slot, agent);
        assert_eq!(ledger, expected, "independent authority for {slot}");
        let stored = crate::effort::stored(document.as_bytes(), slot);
        assert!(
            matches!(stored, Stored::Present(_)),
            "{slot} has applied meta"
        );
        assert_eq!(replay(&stored, &ledger), Resume::Replay(Level::High));
    }
}

#[test]
fn a_worker_only_applied_episode_does_not_require_main_effort_meta() {
    let document = meta(VALID).replace(&format!("effort.main={VALID}\n"), "") + &worker_document();
    assert_eq!(
        crate::effort::stored(document.as_bytes(), "main"),
        Stored::Absent
    );
    let active = worker_history();
    let journal = active.clone() + &padding(1100);
    assert_eq!(
        folded_for(&journal, "worker.0", "colead"),
        expected_worker()
    );
    let kept = retained_document("worker", &document, &journal);
    assert_eq!(kept, active + &newest_window(&journal));
    let ledger = folded_for(&kept, "worker.0", "colead");
    assert_eq!(ledger, expected_worker());
    let stored = crate::effort::stored(document.as_bytes(), "worker.0");
    assert!(
        matches!(stored, Stored::Present(_)),
        "colead has applied meta"
    );
    assert_eq!(replay(&stored, &ledger), Resume::Replay(Level::High));
}
