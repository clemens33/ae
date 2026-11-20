//! #308 bounded retention: live meta pins only its addressed effort evidence.
//! The oracle is an exact kept journal, not the retention predicate.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance fixtures own scratch metadata and journals"
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::effort::{Fold, Level, Record, Resume, Stored, Switch, fold, on_resume};
use crate::events::{Event, SessionKey};

const SID: &str = "11111111-1111-4111-8111-111111111111";
const VALID: &str = "v1 applied slot:main base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:tok-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:custom client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700001000";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ae-effort308-bounded-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).expect("exclusive fixture directory");
        std::fs::write(
            root.join("meta"),
            format!(
                "session=fixture\nsession_id={SID}\nseat.main=lead\nprofile.main=custom\nagent_bin.main=codex\nlaunch_id.main=tok-1\neffort.main={VALID}\nseat.worker.0=colead\nprofile.worker.0=custom\nagent_bin.worker.0=codex\n"
            ),
        )
        .expect("main has applied meta; worker has none");
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn event(actor: &str, action: &str, slot: &str, reference: &str, summary: &str) -> String {
    let agent = if slot == "main" { "lead" } else { "colead" };
    format!(
        "{{\"ts\":\"2023-11-14T22:13:20Z\",\"actor\":\"{actor}\",\"action\":\"{action}\",\"target\":\"{agent}\",\"target_slot\":\"{slot}\",\"target_session\":\"fixture\",\"target_session_id\":\"{SID}\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}\n"
    )
}

fn folded(journal: &str) -> Fold {
    let events: Vec<_> = journal
        .lines()
        .map(|line| Event::parse_line(line).expect("valid fixture event"))
        .collect();
    fold(&events, SessionKey::new("fixture", SID), "main", "lead")
}

fn journals() -> (String, String) {
    let opening = event(
        "watchdog",
        "effort-episode",
        "main",
        "1700000000",
        "threshold 80",
    );
    let reserved = event(
        "watchdog",
        "effort-reserved",
        "main",
        "1700000000-down",
        "xhigh->high",
    );
    let applied = event(
        "watchdog",
        "effort-applied",
        "main",
        "1700000000-down",
        "xhigh->high",
    );
    let proof = opening.clone() + &reserved + &applied;
    let padding = event("human", "memo", "worker.0", "", "ordinary padding");
    // All exclusions follow the live opening and age out of the newest 1000.
    // The other seat has a valid episode, but has no retained effort meta.
    let journal = opening
        + &event(
            "watchdog",
            "effort-episode",
            "worker.0",
            "1700000000",
            "threshold 80",
        )
        + &event(
            "watchdog",
            "memo",
            "main",
            "",
            "addressed non-effort watchdog record",
        )
        + &reserved
        + &event(
            "watchdog",
            "effort-reserved",
            "worker.0",
            "1700000000-down",
            "xhigh->high",
        )
        + &event(
            "human",
            "effort-yielded",
            "main",
            "1700000000",
            "non-watchdog effort action",
        )
        + &applied
        + &event(
            "watchdog",
            "effort-applied",
            "worker.0",
            "1700000000-down",
            "xhigh->high",
        )
        + &event("human", "memo", "main", "", "addressed ordinary history")
        + &padding.repeat(1100);
    (journal, proof + &padding.repeat(1000))
}

#[test]
fn live_meta_does_not_pin_other_seats_or_unrelated_addressed_history() {
    let scratch = Scratch::new();
    let (journal, expected) = journals();
    let original_fold = folded(&journal);
    assert!(matches!(original_fold, Fold::Open(_)));
    let path = scratch.0.join("events.jsonl");
    let original_meta = std::fs::read(scratch.0.join("meta")).expect("original meta");
    assert_eq!(
        crate::effort::stored(&original_meta, "worker.0"),
        Stored::Absent
    );
    std::fs::write(&path, &journal).expect("journal before trim");
    for _ in 0..2 {
        super::trim_events(&scratch.0);
        let kept = std::fs::read_to_string(&path).expect("journal after trim");
        assert_eq!(
            kept, expected,
            "only main's three proof records extend the newest 1000"
        );
        assert_eq!(kept.lines().count(), 1003);
        assert_eq!(
            std::fs::read(scratch.0.join("meta")).expect("meta after trim"),
            original_meta
        );
        let ledger = folded(&kept);
        assert_eq!(
            ledger, original_fold,
            "non-watchdog effort actions are inert"
        );
        assert_eq!(
            on_resume(
                &Stored::Present(Record::parse(VALID).expect("literal record")),
                Ok(()),
                Some(&ledger),
                Switch::On,
                true
            ),
            Resume::Replay(Level::High)
        );
    }
}

#[test]
fn addressed_reseat_and_spawn_resets_cannot_be_erased_beside_stale_applied_meta() {
    // Like a discard publication, a reset can be durable before clearing the
    // old effort meta succeeds. The fold resets for ANY addressed actor.
    for action in ["reseat", "spawn"] {
        let scratch = Scratch::new();
        let opening = event(
            "watchdog",
            "effort-episode",
            "main",
            "1700000000",
            "threshold 80",
        );
        let reserved = event(
            "watchdog",
            "effort-reserved",
            "main",
            "1700000000-down",
            "xhigh->high",
        );
        let applied = event(
            "watchdog",
            "effort-applied",
            "main",
            "1700000000-down",
            "xhigh->high",
        );
        let reset = event(
            "human",
            action,
            "main",
            "",
            "durable reset; meta clear failed",
        );
        let evidence = opening + &reserved + &applied + &reset;
        let padding = event("human", "memo", "worker.0", "", "ordinary padding");
        let journal = evidence.clone() + &padding.repeat(1100);
        assert_eq!(folded(&journal), Fold::Armed, "original reset: {action}");
        let path = scratch.0.join("events.jsonl");
        let original_meta = std::fs::read(scratch.0.join("meta")).expect("original meta");
        std::fs::write(&path, &journal).expect("journal before trim");
        let expected = evidence + &padding.repeat(1000);
        for _ in 0..2 {
            super::trim_events(&scratch.0);
            let kept = std::fs::read_to_string(&path).expect("journal after trim");
            // Live meta requires retaining the episode and every later reset,
            // rather than selectively dropping the reset or all its proof.
            assert_eq!(
                kept, expected,
                "the addressed {action} stays with the proof"
            );
            assert_eq!(kept.lines().count(), 1004);
            let ledger = folded(&kept);
            assert_eq!(ledger, Fold::Armed, "retained reset: {action}");
            assert_eq!(
                on_resume(
                    &Stored::Present(Record::parse(VALID).expect("literal record")),
                    Ok(()),
                    Some(&ledger),
                    Switch::On,
                    true
                ),
                Resume::Keep(crate::effort::Kept::Ledger)
            );
            assert_eq!(
                std::fs::read(scratch.0.join("meta")).expect("meta after trim"),
                original_meta
            );
        }
    }
}
