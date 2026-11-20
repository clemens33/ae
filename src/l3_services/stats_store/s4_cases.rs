//! Frozen S4 registration/crash oracle, shared by tests and stats_disk fuzz.
#![allow(
    clippy::expect_used,
    reason = "contract operations and independent fold assertions"
)]

#[cfg(test)]
use crate as ae;
use ae::stats::{self, Source, ThisSession};
use ae::stats_store::disk::{
    self, Decision, Memory, MemoryJournals, Next, Op, Plan, Prepared, Seen, Shown, SourceSpec, Why,
};

const S: &str = "11111111-1111-4111-8111-111111111111";
const T: &str = "22222222-2222-4222-8222-222222222222";

fn birth(n: u32) -> Vec<u8> {
    format!("{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{S}\",\"birth_v\":1,\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-{n:012x}\",\"birth_tool\":\"codex\",\"birth_profile\":\"preset\",\"birth_model\":\"sol\",\"birth_effort\":\"high\"}}\n").into_bytes()
}

fn plan(decision: Decision) -> Plan {
    match decision {
        Decision::Publish(plan) => plan,
        other => panic!("expected publish: {other:?}"),
    }
}

fn apply(files: &mut Memory, plan: &Plan) {
    plan.apply(files, plan.ops.len()).expect("complete plan");
}

fn recover(files: &mut Memory) {
    let recovery = disk::recovery(files).expect("recoverable crash prefix");
    apply(files, &recovery);
    assert!(
        disk::recovery(files)
            .expect("recovery idempotent")
            .ops
            .is_empty()
    );
}

fn check(files: &mut Memory, bytes: &[u8], present: bool) {
    for scope in [None, Some(S), Some(T)] {
        let expected = stats::fold(
            if present {
                vec![Source::live(
                    "alpha",
                    format!("session=alpha\nsession_id={S}\n").as_bytes(),
                    Ok(bytes.to_vec()),
                )]
            } else {
                Vec::new()
            },
            scope,
        );
        match disk::shown(files, scope) {
            Shown::Ready { view, updating, .. } => {
                assert!(!updating);
                assert_eq!(
                    view, expected,
                    "original fold, every field in scope {scope:?}"
                );
                assert!(view.all.balanced());
                if let ThisSession::Ready(this) = view.this {
                    assert!(this.balanced());
                }
            }
            Shown::Building if !present => {}
            other => panic!("expected durable projection: {other:?}"),
        }
    }
}

fn snapshot(files: &Memory) -> Vec<(String, Vec<u8>)> {
    files
        .names()
        .into_iter()
        .map(|name| {
            let bytes = files.file(&name).expect("listed file").to_vec();
            (name, bytes)
        })
        .collect()
}

pub fn exercise(data: &[u8]) {
    let kind = data.first().copied().unwrap_or_default() % 4;
    let index = usize::from(data.get(1).copied().unwrap_or_default());
    let mut files = Memory::new();
    let mut journals = MemoryJournals::new();
    let spec = SourceSpec::Live {
        id: "live/alpha".to_owned(),
        label: "alpha".to_owned(),
        meta: format!("session=alpha\nsession_id={S}\n").into_bytes(),
    };
    let mut journal = birth(1);
    let seen = Seen {
        dev: 1,
        ino: 10,
        len: journal.len() as u64,
    };
    let first = plan(
        disk::event_counted(&mut files, &spec, 0, &journal[..journal.len() - 1], seen)
            .expect("cold registration"),
    );
    if kind == 0 {
        let cut = index % (first.ops.len() + 1);
        // Independent commit oracle: primary evidence rename, never recovery's answer.
        let committed = first.ops.iter().take(cut).any(|op| matches!(op, Op::Rename { to, .. } if to == &format!("stats/session/{S}.evidence")));
        first
            .apply(&mut files, cut)
            .expect("registration crash prefix");
        let recovery = disk::recovery(&mut files).expect("first recovery plan");
        let redo_cut =
            usize::from(data.get(2).copied().unwrap_or_default()) % (recovery.ops.len() + 1);
        recovery
            .apply(&mut files, redo_cut)
            .expect("second crash in recovery");
        recover(&mut files);
        check(&mut files, &journal, committed);
    } else {
        apply(&mut files, &first);
        check(&mut files, &journal, true);
        let before = snapshot(&files);
        match kind {
            1 => {
                let repeated = plan(
                    disk::event_counted(&mut files, &spec, 0, &journal[..journal.len() - 1], seen)
                        .expect("duplicate event"),
                );
                assert!(repeated.ops.is_empty(), "event insert is idempotent");
            }
            2 => {
                let other = SourceSpec::Live {
                    id: "live/alpha".to_owned(),
                    label: "alpha".to_owned(),
                    meta: format!("session=alpha\nsession_id={T}\n").into_bytes(),
                };
                assert_eq!(
                    disk::event_counted(&mut files, &other, 0, &journal[..journal.len() - 1], seen)
                        .expect("ownership decision"),
                    Decision::Discard(Why::Owned)
                );
            }
            _ => {
                journal = birth(2);
                assert_eq!(
                    disk::event_counted(
                        &mut files,
                        &spec,
                        0,
                        &journal[..journal.len() - 1],
                        Seen {
                            dev: 1,
                            ino: 11,
                            len: journal.len() as u64
                        }
                    )
                    .expect("incarnation decision"),
                    Decision::Discard(Why::Journal)
                );
            }
        }
        assert_eq!(
            snapshot(&files),
            before,
            "duplicate or skipped event writes nothing"
        );
    }
    // Ordered catch-up repairs an uncommitted event and replaces a skipped incarnation.
    let ino = if kind == 3 { 11 } else { 10 };
    journals.set("live/alpha", 1, ino, &journal);
    for _ in 0..8 {
        match Prepared::read(&mut files, &mut journals, &spec, disk::LIMITS, None)
            .expect("catch-up preparation")
        {
            Next::Work(work) => {
                let publish = plan(
                    disk::publish(&mut files, &mut journals, &work).expect("catch-up publication"),
                );
                apply(&mut files, &publish);
            }
            Next::Nothing => {
                check(&mut files, &journal, true);
                return;
            }
            Next::Holding(_) => panic!("fixture lines fit one chunk"),
        }
    }
    panic!("small registration fixture must catch up");
}
