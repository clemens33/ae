//! `ae console` phase 1 — the read view of a session's lead pair.
//!
//! Fixtures reuse the board's synthetic Claude stores: the console reads
//! transcripts through the board's one seat read, so what it must NOT show
//! (a marked turn, a worker's words) is pinned here.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixture setup crosses the filesystem boundary the product observes"
)]

use ae::board::{self, Inputs};
use ae::usage::SessionInput;

use super::board::{claude_roster, plant_session, plant_transcript, rig, user};

const LEAD_ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";
const SCOUT_ID: &str = "0199c0de-1234-4890-abcd-ef0123456790";

#[test]
fn a_seat_filtered_read_yields_lead_human_turns_and_never_a_marked_or_worker_one() {
    let root = rig("console-filtered");
    let store = root.join("claude");
    let marked = ae::provenance::peer("worker");
    let lines = [
        user("2026-09-16T09:00:00Z", "human one"),
        user("2026-09-16T09:01:00Z", &marked),
    ];
    plant_transcript(&store, "work", LEAD_ID, &lines);
    let lines = [user("2026-09-16T09:04:00Z", "worker words")];
    plant_transcript(&store, "work", SCOUT_ID, &lines);
    let roster = format!(
        "{}{}",
        claude_roster("main", "lead", LEAD_ID, &store),
        claude_roster("worker.1", "scout", SCOUT_ID, &store)
    );
    plant_session(&root, "one", &roster);
    let sessions = [SessionInput {
        name: "one".to_owned(),
        path: root.join("sessions").join("one"),
    }];
    let inputs = Inputs {
        home: Some(&root),
        sessions: &sessions,
        assistant: false,
    };
    let bodies = |keep: &dyn Fn(&ae::meta::RosterEntry) -> bool| -> Vec<String> {
        let seen = board::observe_selected(&inputs, None, None, keep);
        seen.rows.into_iter().map(|row| row.body).collect()
    };
    assert_eq!(bodies(&|entry| entry.slot == "main"), ["human one"]);
    assert_eq!(
        bodies(&|_| true),
        ["human one", "worker words"],
        "the filter alone kept the worker out"
    );
    let _ = std::fs::remove_dir_all(&root);
}
