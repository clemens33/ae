#![no_main]

use ae::effort::{
    Decision, Fold, Inputs, Leg, Level, Record, Scope, Stored, Twin, Window, decide, fold,
};
use ae::events::{Event, SessionKey};
use ae::quota::WindowKey;
use libfuzzer_sys::fuzz_target;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const VALID: &str = "v1 applied slot:spawned.0 base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:scout tool:codex launch:launch-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:solx client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700000100";

// A seat's journal folded into its effort episode, then decided against a
// fixed table of scopes, records and live levels. Each line is one
// `events.jsonl` record; a line the journal parser refuses is skipped, because
// the strict damaged-journal ledger is the caller's. No input may panic, the
// fold is a function of its records, and no decision may move effort, open an
// episode or settle one that the fold does not allow.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let events: Vec<Event> = text
        .lines()
        .filter_map(|line| Event::parse_line(line).ok())
        .collect();
    let session = SessionKey::new("seat-session", SID);
    let folded = std::hint::black_box(fold(&events, session, "spawned.0", "scout"));
    assert_eq!(fold(&events, session, "spawned.0", "scout"), folded);
    // A new incarnation of the seat remembers nothing, doubt included.
    let mut reborn = events.clone();
    reborn.push(
        Event::parse_line(&format!(
            r#"{{"ts":"2023-11-14T22:13:20Z","actor":"lead","action":"spawn","target":"scout","target_slot":"spawned.0","target_session":"seat-session","target_session_id":"{SID}"}}"#
        ))
        .expect("a spawn record parses"),
    );
    assert_eq!(fold(&reborn, session, "spawned.0", "scout"), Fold::Armed);
    let open = match &folded {
        Fold::Open(episode) => Some(episode),
        Fold::Armed | Fold::Doubt(_) => None,
    };
    let twin = Ok(Twin {
        baseline: Level::Xhigh,
        target: Level::High,
        aliases: vec!["low".into()],
    });
    let present = Record::parse(VALID).map_or(Stored::Absent, Stored::Present);
    for stored in [Stored::Absent, present] {
        for (judged, capped) in [
            (0.0, false),
            (74.9, false),
            (79.0, false),
            (80.0, false),
            (90.0, true),
        ] {
            let scope = Scope {
                windows: vec![Window {
                    key: WindowKey {
                        bucket: "weekly".into(),
                        qualifier: None,
                        minutes: Some(10_080),
                    },
                    judged,
                    usable: true,
                    observed_at: 1_700_000_000,
                    resets_at: Some(1_700_000_100),
                }],
                spend_capped: capped,
            };
            for live in [Some(Level::Xhigh), Some(Level::High)] {
                let inputs = Inputs {
                    at: 80,
                    new_attempts: true,
                    scope: Some(&scope),
                    limited: false,
                    fold: &folded,
                    stored: &stored,
                    guard: Some(Ok(())),
                    twin: &twin,
                    live,
                    now: 1_700_000_010,
                };
                match std::hint::black_box(decide(&inputs)) {
                    Decision::Down { .. } => assert!(
                        open.is_some_and(|e| e.down == Leg::Unspent && !e.yielded),
                        "a step down needs an unspent, unyielded episode"
                    ),
                    Decision::Up => {
                        assert!(
                            open.is_some_and(
                                |e| matches!(e.down, Leg::Done(_)) && e.up == Leg::Unspent
                            ),
                            "a step up needs a done down and an unspent up"
                        )
                    }
                    Decision::Open { .. } => assert_eq!(folded, Fold::Armed),
                    Decision::Reconcile(_)
                    | Decision::Rearm
                    | Decision::Retire
                    | Decision::Yield(_)
                    | Decision::Name(_) => assert!(open.is_some(), "only an open episode settles"),
                    Decision::Idle | Decision::Hold(_) => {}
                }
            }
        }
    }
});
