//! Frozen banked-resets acceptance r4: a resume trim keeps a live reset-expiry
//! receipt and drops an old one.
//!
//! INSTALL (driver, byte-identical): copy to
//! `src/l4_orchestration/session_launch/banked_trim_spec.rs` and add before the
//! `blocknotice_trim_spec` block in `src/l4_orchestration/session_launch.rs`
//! the triple, as text: `#[cfg(test)]`,
//! `#[path = "session_launch/banked_trim_spec.rs"]`, `mod banked_trim_spec;`
//! Additive to r3 (`b35d929751b3501bbe7756ee8db5f98295183418`); no r3 pin moves.
//! Oracle: brief ruling 4 ("never repeated for the same date", across restarts)
//! and the r3 receipt rule (a `resetexpiry` record by `watchdog` whose reference
//! equals the notice's). A receipt whose reset can still come due must survive
//! the cut, or the next watchdog tells the lead again. One whose instant has
//! passed proves nothing anymore and is cut like any old record.
//!
//! Every reference here comes from `crate::quota::reset_expiry_notices` itself,
//! so the reference format stays the product's. Compile-RED on base by design
//! (new API). Behavioral RED on a candidate: remove the reset-expiry clause from
//! `standing_notice`, and the live receipt is cut.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns and removes only its private fleet roots and journal"
)]

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

const ID: &str = "01a08046-1974-7352-ade3-81a786200795";

/// One weekly `codex` reading, so the declared scope proves its source.
const ROLLOUT: &str = r#"{"timestamp":"2026-09-08T09:09:00.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","limit_name":null,"primary":{"used_percent":10.0,"window_minutes":10080,"resets_at":1789377000},"secondary":null,"plan_type":"pro"}}}
"#;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Root(std::path::PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn root(tag: &str) -> Root {
    let path = std::env::temp_dir().join(format!(
        "ae-banked-trim-{}-{tag}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    Root(path)
}

/// Minute precision with an explicit offset, rounded down.
fn spelled(epoch: i64) -> String {
    let stamp = crate::time::Timestamp::from_epoch(epoch).to_string();
    format!("{}+00:00", &stamp[..16])
}

/// The product's own reference for a reset expiring at `expiry`, observed at
/// `seen` (within 48 h before it), under one private fleet.
fn reference(fleet: &Root, expiry: i64, seen: i64) -> String {
    let shared = fleet.0.join(".codex-shared/sessions/2026/09/08");
    std::fs::create_dir_all(&shared).expect("rollout day");
    std::fs::create_dir_all(fleet.0.join("sessions/session")).expect("session directory");
    std::fs::write(
        shared.join(format!("rollout-2026-09-08T09-00-00-{ID}.jsonl")),
        ROLLOUT,
    )
    .expect("rollout");
    std::fs::write(
        fleet.0.join("config"),
        format!(
            "[clients]\ncx = codex config_home=$HOME/.codex-shared manual_resets={}\n[profiles]\ncxp = cx --model x\n",
            spelled(expiry)
        ),
    )
    .expect("config");
    std::fs::write(
        fleet.0.join("sessions/session/meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main=cxp\nharness_session.main={ID}\nagent_bin.main=codex\n"
        ),
    )
    .expect("meta");
    let observation = crate::quota::observe(&crate::quota::Inputs {
        home: Some(&fleet.0),
        global: Some(&fleet.0.join("config")),
        local: None,
        sessions: Some(&fleet.0.join("sessions")),
        now: seen,
    })
    .expect("the private fleet observes");
    let found = crate::quota::reset_expiry_notices(&observation, &[]);
    assert_eq!(found.len(), 1, "one due reset in the fixture");
    found[0].reference.clone()
}

fn line(actor: &str, reference: &str) -> String {
    let mut escaped = String::new();
    crate::json::escape_into(reference, &mut escaped);
    format!(
        r#"{{"ts":"2026-09-08T09:10:00Z","actor":"{actor}","action":"{}","target":"lead","ref":"{escaped}"}}"#,
        crate::quota::action::RESET_EXPIRY
    )
}

/// A session journal whose first lines are `first`, then more filler than the
/// cut keeps.
fn seed(dir: &std::path::Path, first: &[String]) {
    let mut text = String::new();
    for line in first {
        writeln!(text, "{line}").expect("a journal line");
    }
    for index in 1..=super::EVENTS_KEEP + 5 {
        writeln!(text, "{{\"seq\":{index}}}").expect("a filler line");
    }
    std::fs::write(dir.join("events.jsonl"), text).expect("journal");
}

fn journal(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("events.jsonl")).expect("journal")
}

#[test]
fn a_resume_trim_keeps_a_live_reset_receipt_and_cuts_an_old_one() {
    let now = crate::time::Timestamp::now().epoch();
    // Live: expires in 24 h, seen now. Old: expired 24 h ago, seen 48 h ago.
    let live = reference(&root("live-ref"), now + 86_400, now);
    let old = reference(&root("old-ref"), now - 86_400, now - 2 * 86_400);
    assert_ne!(live, old, "two instants, two receipts");

    let fleet = root("trim");
    let dir = fleet.0.join("sessions").join("s");
    std::fs::create_dir_all(&dir).expect("session dir");
    let kept_line = line("watchdog", &live);
    let old_line = line("watchdog", &old);
    let forged = line("lead", &live);
    seed(&dir, &[old_line.clone(), kept_line.clone(), forged.clone()]);

    super::trim_events(&dir);

    let kept = journal(&dir);
    let head: String = kept.chars().take(600).collect();
    assert!(
        kept.starts_with(&format!("{kept_line}\n")),
        "the live receipt stands first: {head}"
    );
    assert_eq!(
        kept.matches(&kept_line).count(),
        1,
        "kept once, not duplicated: {head}"
    );
    assert!(!kept.contains(&old_line), "an old receipt is cut: {head}");
    assert!(
        !kept.contains(r#""actor":"lead""#),
        "only a watchdog receipt counts as one: {head}"
    );
    assert_eq!(
        kept.lines().count(),
        super::EVENTS_KEEP + 1,
        "the cut keeps its window plus the one standing receipt"
    );
}

#[test]
fn a_reset_receipt_stands_until_the_expiry_instant_and_falls_at_it() {
    // r7: no wall clock. The expiry instant itself is excluded, as for the count.
    const NOW: i64 = 1_788_858_600; // 2026-09-08T09:10:00Z
    const EXPIRY: i64 = NOW + 86_400; // minute-aligned: 2026-09-09T09:10:00Z
    let fleet = root("edge-ref");
    let reference = reference(&fleet, EXPIRY, NOW);
    let parsed = |actor| {
        crate::events::Event::parse_line(&line(actor, &reference)).expect("a receipt parses")
    };
    let booked = parsed("watchdog");
    let stands = |now| crate::quota::reset_expiry_receipt_stands(&booked, now);
    assert!(stands(NOW), "a fresh receipt stands");
    assert!(
        stands(EXPIRY - 1),
        "one second before the instant it stands"
    );
    assert!(!stands(EXPIRY), "at the instant itself the reset is gone");
    assert!(!stands(EXPIRY + 1), "after the instant it is gone");
    assert!(
        !crate::quota::reset_expiry_receipt_stands(&parsed("lead"), EXPIRY - 1),
        "only a watchdog receipt stands"
    );
}
