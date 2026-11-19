//! #308 Gap 2 acceptance, authored by OpenAI/gpt-6.1-sol before implementation.
//! Oracle: brief-fix5 P1 and lead ruling ae-20261113T024347Z-1602448d.
//! Real locked publication, real journal and literal applied-episode evidence.
//! The live test in `effort_live_spec` proves the caller reaches this boundary.
#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures own scratch directories and inspect their files"
)]

use super::{Conversation, SeatMove, publish_seat_move};
use crate::effort::{Fold, Record, Stored};
use crate::events::{Event, SessionKey};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const SID: &str = "11111111-1111-4111-8111-111111111111";
const CONV: &str = "22222222-2222-4222-8222-222222222222";
const VALID: &str = "v1 applied slot:main base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:lead tool:claude launch:old-token pid:42 conv:22222222-2222-4222-8222-222222222222 profile:opus55x client:- model:opus win:weekly/~/10080/1700000000/1700000100";
const OTHER: &str =
    "effort.worker.0=other-seat-claim\nseat.worker.0=colead\nprofile.worker.0=sol\n";
static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(rows: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "ae-effort-reseat-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let dir = root.join("fixture");
        std::fs::create_dir_all(&dir).expect("owned acceptance directory");
        let fixture = Self(root);
        std::fs::write(
            dir.join("meta"),
            format!(
                "session=fixture\nsession_id={SID}\nseat.main=lead\nprofile.main=opus55x\nagent_bin.main=claude\nlaunch_id.main=old-token\nharness_session.main={CONV}\nconfig_home.main=/account\ncapture_floor.main=1600000000\n{OTHER}{rows}"
            ),
        )
        .expect("literal meta");
        std::fs::write(dir.join("events.jsonl"), applied_trace()).expect("literal applied trace");
        fixture
    }

    fn dir(&self) -> PathBuf {
        self.0.join("fixture")
    }

    fn meta(&self) -> String {
        std::fs::read_to_string(self.dir().join("meta")).expect("fixture meta")
    }

    fn events(&self) -> Vec<Event> {
        std::fs::read_to_string(self.dir().join("events.jsonl"))
            .expect("fixture journal")
            .lines()
            .map(|line| Event::parse_line(line).expect("every emitted journal record parses"))
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn applied_trace() -> String {
    [
        ("effort-episode", "1700000000", "threshold 80"),
        ("effort-reserved", "1700000000-down", "xhigh->high"),
        ("effort-applied", "1700000000-down", "xhigh->high"),
    ]
    .map(|(action, reference, summary)| {
        format!(
            "{{\"ts\":\"2023-11-14T22:13:20Z\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"lead\",\"target_slot\":\"main\",\"target_session\":\"fixture\",\"target_session_id\":\"{SID}\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}\n"
        )
    })
    .concat()
}

fn discard_line(reference: &str) -> String {
    format!(
        "{{\"ts\":\"2023-11-14T22:13:21Z\",\"actor\":\"watchdog\",\"action\":\"effort-discarded\",\"target\":\"lead\",\"target_slot\":\"main\",\"target_session\":\"fixture\",\"target_session_id\":\"{SID}\",\"ref\":\"{reference}\",\"summary\":\"reseat\"}}\n"
    )
}

fn assert_seeded_retry(rows: &str, reference: &str) {
    let journal = format!("{}{}", applied_trace(), discard_line(reference));
    assert_seeded_retry_with_trace(rows, reference, &journal);
}

fn stop_line() -> &'static str {
    "{\"ts\":\"2023-11-14T22:13:22Z\",\"actor\":\"human\",\"action\":\"reseat\",\"target\":\"lead\",\"target_slot\":\"main\",\"summary\":\"stopped claude in place (pane %0) [from opus55x to new-profile]\"}\n"
}

fn assert_seeded_retry_with_trace(rows: &str, reference: &str, journal: &str) {
    let fixture = Fixture::new(rows);
    std::fs::write(fixture.dir().join("events.jsonl"), journal)
        .expect("append committed before the failed meta publish");
    publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    )
    .expect("retry publishes the still-held claim");
    assert!(matches!(
        crate::effort::stored(fixture.meta().as_bytes(), "main"),
        Stored::Absent
    ));
    assert_eq!(
        std::fs::read_to_string(fixture.dir().join("events.jsonl")).expect("retry journal"),
        journal,
        "a committed outcome is reused without a second append"
    );
    assert_discard(&fixture, reference);
}

#[test]
fn effort_reseat_a_real_stop_record_does_not_replace_the_effort_outcome() {
    let fixture = applied();
    std::fs::write(
        fixture.dir().join("events.jsonl"),
        format!("{}{}", applied_trace(), stop_line()),
    )
    .expect("the real caller records its stop before publication");
    publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    )
    .expect("reseat publication after a stop");
    assert_discard(&fixture, "1700000000");
}

#[test]
fn effort_reseat_retry_reuses_its_outcome_across_another_reseat_stop_record() {
    let journal = format!(
        "{}{}{}",
        applied_trace(),
        discard_line("1700000000"),
        stop_line()
    );
    assert_seeded_retry_with_trace(&format!("effort.main={VALID}\n"), "1700000000", &journal);
}

fn assert_unterminated_tail(tail: &str, committed: usize) {
    let fixture = applied();
    let journal = format!("{}{tail}", applied_trace());
    let path = fixture.dir().join("events.jsonl");
    std::fs::write(&path, &journal).expect("crash left an unterminated tail");
    let before = fixture.meta();
    // The actual watchdog consumer excludes an unterminated tail. A parser
    // over String::lines would see a complete JSON tail too early.
    let consumed = crate::watchdog_daemon::read_events(&fixture.dir());
    assert_eq!(
        consumed
            .iter()
            .filter(|event| event.action == "effort-discarded")
            .count(),
        committed,
        "the complete-record precondition must apply: {tail:?}"
    );
    let fold = crate::effort::fold(&consumed, SessionKey::new("fixture", SID), "main", "lead");
    assert!(
        matches!((committed, fold), (0, Fold::Open(_)) | (1, Fold::Armed)),
        "the consumer proves the pre-publication state: {tail:?}"
    );
    let result = publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    );
    if result.is_err() {
        assert_eq!(fixture.meta(), before, "refusal keeps the applied claim");
        assert_eq!(
            std::fs::read_to_string(path).expect("refused journal"),
            journal
        );
    } else {
        let consumed = crate::watchdog_daemon::read_events(&fixture.dir());
        assert_eq!(
            consumed
                .iter()
                .filter(|event| event.action == "effort-discarded")
                .count(),
            1,
            "a successful publication requires one consumer-visible ending: {tail:?}"
        );
        assert_eq!(
            crate::effort::fold(&consumed, SessionKey::new("fixture", SID), "main", "lead"),
            Fold::Armed,
            "the consumer must accept the ending before the claim disappears: {tail:?}"
        );
        assert!(matches!(
            crate::effort::stored(fixture.meta().as_bytes(), "main"),
            Stored::Absent
        ));
    }
}

#[test]
fn effort_reseat_an_unterminated_ending_cannot_authorize_losing_the_claim() {
    assert_unterminated_tail(discard_line("1700000000").trim_end_matches('\n'), 0);
}

#[test]
fn effort_reseat_a_torn_tail_cannot_swallow_the_new_ending() {
    assert_unterminated_tail("{\"ts\":\"2023-11-14T22:13:21Z\",\"actor\":\"watch", 0);
}

#[test]
fn effort_reseat_a_torn_stop_after_an_ending_cannot_duplicate_it() {
    assert_unterminated_tail(
        &format!(
            "{}{{\"ts\":\"2023-11-14T22:13:22Z\",\"actor\":\"human",
            discard_line("1700000000")
        ),
        1,
    );
}

#[test]
fn effort_reseat_an_unterminated_stop_after_an_ending_cannot_duplicate_it() {
    assert_unterminated_tail(
        &format!(
            "{}{}",
            discard_line("1700000000"),
            stop_line().trim_end_matches('\n')
        ),
        1,
    );
}

#[test]
fn effort_reseat_an_old_empty_ref_ending_does_not_close_a_newer_damaged_claim() {
    let fixture = Fixture::new("effort.main=damaged\r\n");
    std::fs::write(
        fixture.dir().join("events.jsonl"),
        format!("{}{}", discard_line(""), applied_trace()),
    )
    .expect("a new episode follows the older damaged claim ending");
    publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    )
    .expect("the newer damaged claim closes");
    let discarded: Vec<_> = fixture
        .events()
        .into_iter()
        .filter(|event| event.action == "effort-discarded")
        .collect();
    assert_eq!(
        discarded.len(),
        2,
        "the old empty-ref ending cannot close a new claim"
    );
    assert!(discarded.iter().all(
        |event| event.reference.as_deref().unwrap_or_default().is_empty()
            && event.summary.as_deref() == Some("reseat")
    ));
}

#[test]
fn effort_reseat_retry_after_a_committed_discard_reuses_that_exact_outcome() {
    assert_seeded_retry(&format!("effort.main={VALID}\n"), "1700000000");
}

#[test]
fn effort_reseat_retry_of_a_damaged_claim_reuses_its_empty_reference_outcome() {
    assert_seeded_retry("effort.main=damaged\r\n", "");
}

#[test]
fn effort_reseat_an_older_discard_does_not_close_a_newer_applied_claim() {
    let fixture = applied();
    std::fs::write(
        fixture.dir().join("events.jsonl"),
        format!("{}{}", discard_line("1699999999"), applied_trace()),
    )
    .expect("an older ending precedes the new applied episode");
    assert!(matches!(
        crate::effort::fold(
            &fixture.events(),
            SessionKey::new("fixture", SID),
            "main",
            "lead"
        ),
        Fold::Open(_)
    ));
    publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    )
    .expect("new episode closes");
    let discarded: Vec<_> = fixture
        .events()
        .into_iter()
        .filter(|event| event.action == "effort-discarded")
        .collect();
    assert_eq!(discarded.len(), 2, "both distinct episodes have an ending");
    assert_eq!(discarded[1].reference.as_deref(), Some("1700000000"));
    assert_eq!(discarded[1].summary.as_deref(), Some("reseat"));
}

fn ending_with_reason(reference: &str, reason: &str) -> String {
    let summary = format!("\"summary\":\"{reason}\"");
    let line = discard_line(reference).replace("\"summary\":\"reseat\"", &summary);
    assert!(line.contains(&summary), "the fixture must name its reason");
    line
}

fn assert_reuses_other_path_ending(reason: &str) {
    let fixture = applied();
    let journal = format!(
        "{}{}",
        applied_trace(),
        ending_with_reason("1700000000", reason)
    );
    std::fs::write(fixture.dir().join("events.jsonl"), &journal)
        .expect("another path journaled its ending before meta publication failed");
    let consumed = crate::watchdog_daemon::read_events(&fixture.dir());
    assert_eq!(consumed.len(), 4, "the literal ending is consumer-visible");
    assert_eq!(
        crate::effort::fold(&consumed, SessionKey::new("fixture", SID), "main", "lead"),
        Fold::Armed,
        "the real consumer accepts the other path's ending"
    );
    assert!(matches!(
        crate::effort::stored(fixture.meta().as_bytes(), "main"),
        Stored::Present(_)
    ));
    publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    )
    .expect("reseat republishes after another path's failed publication");
    assert!(matches!(
        crate::effort::stored(fixture.meta().as_bytes(), "main"),
        Stored::Absent
    ));
    assert_eq!(
        std::fs::read_to_string(fixture.dir().join("events.jsonl")).expect("retry journal"),
        journal,
        "an applied episode already has its one ending, regardless of the path"
    );
    assert_eq!(
        crate::effort::fold(
            &crate::watchdog_daemon::read_events(&fixture.dir()),
            SessionKey::new("fixture", SID),
            "main",
            "lead"
        ),
        Fold::Armed
    );
}

#[test]
fn effort_reseat_reuses_a_fresh_ending_after_its_meta_publication_failed() {
    assert_reuses_other_path_ending("fresh");
}

#[test]
fn effort_reseat_reuses_an_off_ending_after_its_meta_publication_failed() {
    assert_reuses_other_path_ending("off");
}

fn assert_unmatched_ending_cannot_close_claim(reference: &str, reason: &str, expected: &Fold) {
    let fixture = applied();
    let journal = format!(
        "{}{}",
        applied_trace(),
        ending_with_reason(reference, reason)
    );
    std::fs::write(fixture.dir().join("events.jsonl"), journal)
        .expect("the nonmatching ending follows this episode's opening");
    let consumed = crate::watchdog_daemon::read_events(&fixture.dir());
    assert_eq!(
        consumed.len(),
        4,
        "the negative control is consumer-visible"
    );
    assert_eq!(
        &crate::effort::fold(&consumed, SessionKey::new("fixture", SID), "main", "lead"),
        expected
    );
    publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    )
    .expect("the unmatched claim still needs its own valid ending");
    let consumed = crate::watchdog_daemon::read_events(&fixture.dir());
    let endings: Vec<_> = consumed
        .iter()
        .filter(|event| event.action == "effort-discarded")
        .collect();
    assert_eq!(
        endings.len(),
        2,
        "the earlier record cannot close this claim"
    );
    let actual = endings[1];
    assert_eq!(actual.actor, "watchdog");
    assert_eq!(actual.reference.as_deref(), Some("1700000000"));
    assert_eq!(actual.summary.as_deref(), Some("reseat"));
    assert!(
        SessionKey::new("fixture", SID)
            .seat("main")
            .matches(actual.target_identity().expect("routed ending"))
    );
    assert_eq!(
        crate::effort::fold(&consumed, SessionKey::new("fixture", SID), "main", "lead"),
        Fold::Armed
    );
    assert!(matches!(
        crate::effort::stored(fixture.meta().as_bytes(), "main"),
        Stored::Absent
    ));
}

#[test]
fn effort_reseat_a_late_old_ref_ending_cannot_close_the_current_claim() {
    // The fold is already Armed because any well-formed discard resets it.
    // This pins claim-to-ref binding even when fold state alone cannot do so.
    assert_unmatched_ending_cannot_close_claim("1699999999", "reseat", &Fold::Armed);
}

#[test]
fn effort_reseat_an_invalid_reason_cannot_close_the_current_claim() {
    assert_unmatched_ending_cannot_close_claim(
        "1700000000",
        "unrecognized",
        &Fold::Doubt("unreadable effort discard"),
    );
}

#[test]
fn effort_reseat_a_matching_ref_for_another_identity_cannot_suppress_this_outcome() {
    let original = discard_line("1700000000");
    for unrelated in [
        original.replace("\"actor\":\"watchdog\"", "\"actor\":\"peer\""),
        original.replace("\"target_slot\":\"main\"", "\"target_slot\":\"worker.0\""),
        original.replace(SID, "33333333-3333-4333-8333-333333333333"),
    ] {
        let fixture = applied();
        std::fs::write(
            fixture.dir().join("events.jsonl"),
            format!("{}{unrelated}", applied_trace()),
        )
        .expect("the same ref appears on another identity");
        publish_seat_move(
            &fixture.dir(),
            "main",
            "lead",
            &destination(Conversation::Kept),
        )
        .expect("this seat still needs an outcome");
        let discarded: Vec<_> = fixture
            .events()
            .into_iter()
            .filter(|event| event.action == "effort-discarded")
            .collect();
        assert_eq!(
            discarded.len(),
            2,
            "a foreign ending cannot close this claim"
        );
        let actual = discarded.last().expect("this seat's new ending");
        assert_eq!(actual.actor, "watchdog");
        assert_eq!(actual.reference.as_deref(), Some("1700000000"));
        assert!(
            SessionKey::new("fixture", SID)
                .seat("main")
                .matches(actual.target_identity().expect("routed ending"))
        );
    }
}

/// Non-root executes the real append-ok / publish-fails transition. Root can
/// write through mode 0555, so the seeded crash-state fixture carries its proof.
/// Both branches assert the uid and probe result, and both exercise deduplication.
#[test]
fn effort_reseat_permission_fault_keeps_the_claim_and_retry_does_not_append_twice() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    struct RestoreMode(PathBuf);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755))
                .expect("restore owned fixture permissions");
        }
    }
    let fixture = applied();
    let dir = fixture.dir();
    let uid = std::fs::metadata(dir.join("meta"))
        .expect("fixture uid")
        .uid();
    let before = fixture.meta();
    // Existing lock files remain openable when their directory denies creates.
    for name in ["meta.lock", "events.jsonl.lock"] {
        std::fs::write(dir.join(name), "").expect("pre-existing writable lock file");
    }
    let restore = RestoreMode(dir.clone());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
        .expect("deny directory creation and rename");
    let probe = std::fs::write(dir.join("permission-probe"), "probe");
    if uid == 0 {
        assert_eq!(uid, 0, "root branch");
        assert!(
            probe.is_ok(),
            "root must prove the fault would be ineffective"
        );
        drop(restore);
        assert_seeded_retry(&format!("effort.main={VALID}\n"), "1700000000");
        return;
    }
    assert_ne!(uid, 0, "permission fault branch");
    assert!(
        matches!(probe, Err(ref why) if why.kind() == std::io::ErrorKind::PermissionDenied),
        "directory permission fault must actually apply: {probe:?}"
    );
    let failed = publish_seat_move(&dir, "main", "lead", &destination(Conversation::Kept));
    drop(restore);
    assert!(
        failed.is_err(),
        "temp creation or rename must fail after append"
    );
    assert_eq!(fixture.meta(), before, "failed publication keeps the claim");
    assert_discard(&fixture, "1700000000");
    publish_seat_move(&dir, "main", "lead", &destination(Conversation::Kept))
        .expect("retry after write permission returns");
    assert!(matches!(
        crate::effort::stored(fixture.meta().as_bytes(), "main"),
        Stored::Absent
    ));
    assert_discard(&fixture, "1700000000");
}

fn destination(conversation: Conversation<'_>) -> SeatMove<'_> {
    SeatMove {
        profile: "new-profile",
        binary: "claude",
        launch_id: "new-token",
        conversation,
        pair: None,
    }
}

fn applied() -> Fixture {
    Fixture::new(&format!("effort.main={VALID}\n"))
}

fn assert_discard(fixture: &Fixture, reference: &str) {
    let events = fixture.events();
    let discarded: Vec<_> = events
        .iter()
        .filter(|event| event.action == "effort-discarded")
        .collect();
    assert_eq!(
        discarded.len(),
        1,
        "exactly one reasoned ending: {events:?}"
    );
    let event = discarded[0];
    assert_eq!(event.actor, "watchdog");
    assert_eq!(event.target.as_deref(), Some("lead"));
    assert_eq!(event.reference.as_deref().unwrap_or_default(), reference);
    assert_eq!(event.summary.as_deref(), Some("reseat"));
    assert!(
        SessionKey::new("renamed", SID)
            .seat("main")
            .matches(event.target_identity().expect("routed discard"))
    );
    assert!(events.iter().all(|event| event.action != "effort-restored"));
    assert_eq!(
        crate::effort::fold(&events, SessionKey::new("fixture", SID), "main", "lead"),
        Fold::Armed,
        "the emitted ending is accepted by the real fold"
    );
}

#[test]
fn effort_reseat_fixture_is_a_proven_applied_episode_before_the_move() {
    let fixture = applied();
    assert!(
        matches!(crate::effort::stored(fixture.meta().as_bytes(), "main"), Stored::Present(record) if record == Record::parse(VALID).expect("literal valid record"))
    );
    assert!(
        matches!(crate::effort::fold(&fixture.events(), SessionKey::new("fixture", SID), "main", "lead"), Fold::Open(episode) if matches!(episode.down, crate::effort::Leg::Done(_)))
    );
}

#[test]
fn effort_reseat_every_conversation_arm_journals_one_discard_before_losing_the_claim() {
    for conversation in [
        Conversation::Fresh {
            id: "33333333-3333-4333-8333-333333333333",
            capture_floor: 1_700_000_100,
        },
        Conversation::Carried {
            config_home: "/target-account",
            config_home_base: None,
        },
        Conversation::Kept,
    ] {
        let fixture = applied();
        publish_seat_move(&fixture.dir(), "main", "lead", &destination(conversation))
            .expect("successful publication");
        assert!(matches!(
            crate::effort::stored(fixture.meta().as_bytes(), "main"),
            Stored::Absent
        ));
        assert!(fixture.meta().contains("profile.main=new-profile\n"));
        assert!(
            fixture.meta().contains(OTHER),
            "unrelated seat stays byte-identical"
        );
        assert_discard(&fixture, "1700000000");
    }
}

#[test]
fn effort_reseat_append_failure_keeps_the_entire_meta_and_retry_closes_once() {
    let fixture = applied();
    let before = fixture.meta();
    let journal = fixture.dir().join("events.jsonl");
    std::fs::remove_file(&journal).expect("replace journal with an unwritable shape");
    std::fs::create_dir(&journal).expect("append must refuse a directory");
    let failed = publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    );
    assert!(failed.is_err(), "journal refusal must refuse publication");
    assert_eq!(
        fixture.meta(),
        before,
        "old profile, seat and effort claim survive"
    );
    std::fs::remove_dir(&journal).expect("restore journal availability");
    std::fs::write(&journal, applied_trace()).expect("original trace");
    publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    )
    .expect("retry publishes");
    assert_discard(&fixture, "1700000000");
}

#[test]
fn effort_reseat_a_repeat_without_a_claim_does_not_duplicate_the_outcome() {
    let fixture = applied();
    for _ in 0..2 {
        publish_seat_move(
            &fixture.dir(),
            "main",
            "lead",
            &destination(Conversation::Kept),
        )
        .expect("idempotent move");
    }
    assert_discard(&fixture, "1700000000");
}

#[test]
fn effort_reseat_without_an_effort_claim_preserves_the_journal_exactly() {
    let fixture = Fixture::new("");
    let before = fixture.events();
    publish_seat_move(
        &fixture.dir(),
        "main",
        "lead",
        &destination(Conversation::Kept),
    )
    .expect("ordinary reseat");
    assert_eq!(fixture.events(), before);
    assert!(fixture.meta().contains("profile.main=new-profile\n"));
}

#[test]
fn effort_reseat_a_different_seat_keeps_both_meta_and_journal() {
    let fixture = applied();
    let before = (fixture.meta(), fixture.events());
    publish_seat_move(
        &fixture.dir(),
        "main",
        "successor",
        &destination(Conversation::Kept),
    )
    .expect("guard misses without a write");
    assert_eq!((fixture.meta(), fixture.events()), before);
}

#[test]
fn effort_reseat_damaged_claims_end_with_an_empty_episode_reference() {
    for rows in [
        "effort.main\n".to_owned(),
        "effort.main=damaged\r\n".to_owned(),
        format!("effort.main={VALID}\neffort.main={VALID}\n"),
    ] {
        let fixture = Fixture::new(&rows);
        publish_seat_move(
            &fixture.dir(),
            "main",
            "lead",
            &destination(Conversation::Kept),
        )
        .expect("damaged-claim reseat");
        assert!(matches!(
            crate::effort::stored(fixture.meta().as_bytes(), "main"),
            Stored::Absent
        ));
        assert_discard(&fixture, "");
    }
}
