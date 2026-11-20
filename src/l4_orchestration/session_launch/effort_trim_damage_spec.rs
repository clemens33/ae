//! #308 review supplements: retention preserves existing uncertainty.
//! Independent oracles: strict resume ledger, effort fold authority, and
//! best-effort retention must not destroy proof when its meta read fails.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance fixtures own scratch metadata and journals"
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::effort::{Fold, Kept, Level, Record, Resume, Stored, Switch, fold, on_resume};
use crate::events::SessionKey;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const VALID: &str = "v1 applied slot:main base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:tok-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:custom client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700001000";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ae-effort308-damage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).expect("exclusive fixture directory");
        std::fs::write(
            root.join("meta"),
            format!(
                "session=fixture\nsession_id={SID}\nseat.main=lead\nprofile.main=custom\nagent_bin.main=codex\nlaunch_id.main=tok-1\neffort.main={VALID}\n"
            ),
        )
        .expect("applied meta");
        Self(root)
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

fn active() -> String {
    event("effort-episode", "1700000000", "threshold 80")
        + &event("effort-reserved", "1700000000-down", "xhigh->high")
        + &event("effort-applied", "1700000000-down", "xhigh->high")
}

fn aged(subject: &str) -> String {
    subject.to_owned() + &event("memo", "", "ordinary").repeat(1100)
}

fn choice(journal: &str) -> (Option<Fold>, Resume) {
    // Use the production strict reader, which rejects any damaged nonblank
    // line. A lossy Event filter would hide the defect under review.
    let ledger =
        crate::autotrust::ledger(crate::store::SourceRead::Ready(journal.as_bytes().to_vec()))
            .ok()
            .map(|events| fold(&events, SessionKey::new("fixture", SID), "main", "lead"));
    let stored = Stored::Present(Record::parse(VALID).expect("literal applied record"));
    let resume = on_resume(&stored, Ok(()), ledger.as_ref(), Switch::On, true);
    (ledger, resume)
}

fn trim(journal: &str) -> String {
    let scratch = Scratch::new();
    let path = scratch.0.join("events.jsonl");
    std::fs::write(&path, journal).expect("original journal");
    super::trim_events(&scratch.0);
    std::fs::read_to_string(path).expect("trimmed journal")
}

#[test]
fn semantic_damage_before_the_live_opening_does_not_become_replay_authority() {
    let journal = aged(&(event("effort-held", "1700000001", "unreadable") + &active()));
    let expected = (
        Some(Fold::Doubt("effort record names no open episode")),
        Resume::Keep(Kept::Ledger),
    );
    assert_eq!(choice(&journal), expected, "original authority is doubtful");
    let kept = trim(&journal);
    assert_eq!(choice(&kept), expected, "retention must preserve doubt");
}

#[test]
fn nonblank_invalid_json_does_not_disappear_beside_pinned_applied_proof() {
    // A blank line is ignored by the strict reader and is a neutral control.
    for (damage, readable) in [("\n", true), ("{damaged\n", false)] {
        let journal = aged(&(active() + damage));
        let before = choice(&journal);
        if readable {
            assert_eq!(before.1, Resume::Replay(Level::High));
        } else {
            assert_eq!(before, (None, Resume::Keep(Kept::Ledger)));
        }
        let kept = trim(&journal);
        assert_eq!(choice(&kept), before, "damage={damage:?}");
    }
}

#[test]
fn unavailable_meta_skips_optional_retention_without_destroying_episode_proof() {
    let scratch = Scratch::new();
    let journal = aged(&active());
    let path = scratch.0.join("events.jsonl");
    std::fs::write(&path, &journal).expect("original journal");
    // Deterministic read error on both platforms. The saved applied meta
    // stands intact and is restored after the unavailable-read interval.
    std::fs::rename(scratch.0.join("meta"), scratch.0.join("saved-meta"))
        .expect("preserve original meta");
    std::fs::create_dir(scratch.0.join("meta")).expect("unavailable meta node");
    assert!(crate::meta::read_bytes(&scratch.0).is_err());
    super::trim_events(&scratch.0);
    let kept = std::fs::read_to_string(&path).expect("journal after unavailable meta");
    std::fs::remove_dir(scratch.0.join("meta")).expect("end unavailable interval");
    std::fs::rename(scratch.0.join("saved-meta"), scratch.0.join("meta"))
        .expect("restore original meta");
    assert_eq!(kept, journal, "optional retention must leave the log alone");
    assert_eq!(choice(&kept).1, Resume::Replay(Level::High));
}
