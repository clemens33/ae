//! Frozen banked-resets acceptance: the 48 h reset-expiry notice for the session lead.
//!
//! INSTALL (driver, byte-identical): copy to
//! `src/l4_orchestration/watchdog_daemon/banked_notice_spec.rs` and add after the
//! `activity225_spec` block (rustfmt order) in `src/l4_orchestration/watchdog_daemon.rs`
//! the triple, as text: `#[cfg(test)]`,
//! `#[path = "watchdog_daemon/banked_notice_spec.rs"]`, `mod banked_notice_spec;`
//! Oracle: `.local/brief-banked.md` ruling 4 and the seam the driver named in
//! ae-20261127T093221Z-6aa6581a: `crate::quota::reset_expiry_notices(&Observation,
//! &[Event]) -> Vec<ResetExpiryNotice>` with `reference: String` and `text: String`;
//! a `resetexpiry` event by `watchdog` whose reference equals a notice's reference
//! is its durable receipt; the window is `0 < expiry - now <= 172_800`.
//!
//! New API: this file is compile-RED on the base commit by design.
//!
//! Clock: `NOW` = `1_788_858_600` = `2026-09-08T09:10:00Z`; each date is NOW plus a
//! stated number of seconds, at minute precision with an explicit offset.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns and removes only its private config, fleet meta and rollout"
)]

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::events::Event;

const NOW: i64 = 1_788_858_600;
const ID: &str = "01a08046-1974-7352-ade3-81a786200795";

/// NOW - 1 h.
const PAST: &str = "2026-09-08T08:10+00:00";
/// NOW exactly: already expired, never noticed.
const AT_NOW: &str = "2026-09-08T09:10+00:00";
/// NOW + 1 min.
const IN_1M: &str = "2026-09-08T09:11+00:00";
/// NOW + 24 h, spelled two ways.
const IN_24H_UTC: &str = "2026-09-09T09:10+00:00";
const IN_24H_PLUS_ONE: &str = "2026-09-09T10:10+01:00";
/// NOW + 47 h.
const IN_47H: &str = "2026-09-10T08:10+00:00";
/// NOW + 48 h exactly: inside the window.
const IN_48H: &str = "2026-09-10T09:10+00:00";
/// NOW + 48 h 1 min: outside the window.
const IN_48H_1M: &str = "2026-09-10T09:11+00:00";
/// NOW + 3 d.
const IN_3D: &str = "2026-09-11T09:10+00:00";

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Root(std::path::PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One weekly `codex` reading at NOW - 1 min, so the scope proves its source.
const ROLLOUT: &str = r#"{"timestamp":"2026-09-08T09:09:00.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","limit_name":null,"primary":{"used_percent":10.0,"window_minutes":10080,"resets_at":1789377000},"secondary":null,"plan_type":"pro"}}}
"#;

/// A private fleet root, removed on drop.
fn root() -> Root {
    Root(std::env::temp_dir().join(format!(
        "ae-banked-notice-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )))
}

/// Observe the private fleet under `root` whose `[clients]` rows are `clients`
/// (each label followed by its tail), every row on one shared config home.
/// The rows are rewritten on each call; the config home stays the same path.
fn observe(root: &Root, clients: &[(&str, &str)]) -> crate::quota::Observation {
    let shared = root.0.join(".codex-shared/sessions/2026/09/08");
    std::fs::create_dir_all(&shared).expect("rollout day");
    std::fs::create_dir_all(root.0.join("sessions/session")).expect("session directory");
    std::fs::write(
        shared.join(format!("rollout-2026-09-08T09-00-00-{ID}.jsonl")),
        ROLLOUT,
    )
    .expect("rollout");
    let mut config = String::from("[clients]\n");
    let mut profiles = String::from("[profiles]\n");
    for (label, tail) in clients {
        writeln!(
            config,
            "{label} = codex config_home=$HOME/.codex-shared{tail}"
        )
        .expect("a config row");
        writeln!(profiles, "{label}p = {label} --model x").expect("a profile row");
    }
    std::fs::write(root.0.join("config"), config + &profiles).expect("config");
    let first = clients.first().map_or("none", |(label, _)| label);
    std::fs::write(
        root.0.join("sessions/session/meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main={first}p\nharness_session.main={ID}\nagent_bin.main=codex\n"
        ),
    )
    .expect("meta");
    crate::quota::observe(&crate::quota::Inputs {
        home: Some(&root.0),
        global: Some(&root.0.join("config")),
        local: None,
        sessions: Some(&root.0.join("sessions")),
        now: NOW,
    })
    .expect("the private fleet observes")
}

fn dated(dates: &[&str]) -> String {
    format!(" manual_resets={}", dates.join(","))
}

/// A journal line: `actor` journaled `action` with `reference` (wire key `ref`).
fn receipt(actor: &str, action: &str, reference: &str) -> Event {
    let mut escaped = String::new();
    crate::json::escape_into(reference, &mut escaped);
    let ts = crate::time::Timestamp::from_epoch(NOW - 30);
    let event = Event::parse_line(&format!(
        r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}","ref":"{escaped}"}}"#
    ))
    .expect("a journal line");
    assert_eq!(
        (
            event.actor.as_str(),
            event.action.as_str(),
            event.reference.as_deref()
        ),
        (actor, action, Some(reference)),
        "the fixture line parses back to the receipt it means"
    );
    event
}

#[test]
fn banked_notice_covers_exactly_the_resets_that_expire_within_48_hours() {
    for (tag, dates, expected) in [
        ("past-and-now", &[PAST, AT_NOW][..], 0),
        ("one-minute", &[IN_1M][..], 1),
        ("edges", &[IN_48H, IN_48H_1M][..], 1),
        ("mixed", &[PAST, AT_NOW, IN_47H, IN_48H_1M, IN_3D][..], 1),
        ("two-inside", &[IN_24H_UTC, IN_47H][..], 2),
    ] {
        let observation = observe(&root(), &[("cx", &dated(dates))]);
        let found = crate::quota::reset_expiry_notices(&observation, &[]);
        assert_eq!(
            found.len(),
            expected,
            "{tag}: one notice per declared reset with 0 < expiry - now <= 48 h: {:?}",
            found.iter().map(|notice| &notice.text).collect::<Vec<_>>()
        );
        let mut references: Vec<&str> = found
            .iter()
            .map(|notice| notice.reference.as_str())
            .collect();
        references.sort_unstable();
        references.dedup();
        assert_eq!(
            references.len(),
            expected,
            "{tag}: each reset has its own reference"
        );
        for notice in &found {
            assert!(!notice.reference.is_empty(), "{tag}: a usable receipt key");
            let text = notice.text.to_lowercase();
            assert!(
                text.contains("banked reset") && text.contains("expires"),
                "{tag}: the lead reads what expires: {}",
                notice.text
            );
        }
    }
}

#[test]
fn banked_notice_is_never_given_for_an_undated_refused_or_absent_declaration() {
    for (tag, tail) in [
        ("undated", " manual_resets=2".to_owned()),
        ("refused", format!(" manual_resets={}", "2026-09-09T09:10")),
        ("absent", String::new()),
    ] {
        let observation = observe(&root(), &[("cx", &tail)]);
        assert!(
            crate::quota::reset_expiry_notices(&observation, &[]).is_empty(),
            "{tag}: no date, no notice"
        );
    }
}

#[test]
fn banked_notice_is_given_once_and_its_watchdog_receipt_suppresses_it_for_good() {
    let observation = observe(&root(), &[("cx", &dated(&[IN_24H_UTC, IN_47H]))]);
    let found = crate::quota::reset_expiry_notices(&observation, &[]);
    assert_eq!(found.len(), 2, "two resets inside the window");
    let first = found[0].reference.clone();
    let second = found[1].reference.clone();

    let journal = [receipt("watchdog", "resetexpiry", &first)];
    let left = crate::quota::reset_expiry_notices(&observation, &journal);
    assert_eq!(
        left.iter()
            .map(|notice| notice.reference.as_str())
            .collect::<Vec<_>>(),
        [second.as_str()],
        "the receipt suppresses its own reset and no other"
    );

    let both = [
        receipt("watchdog", "resetexpiry", &first),
        receipt("watchdog", "resetexpiry", &second),
    ];
    assert!(
        crate::quota::reset_expiry_notices(&observation, &both).is_empty(),
        "never repeated for the same date"
    );

    for (tag, journal) in [
        (
            "another action",
            [receipt("watchdog", "quota-advisory", &first)],
        ),
        ("another actor", [receipt("lead", "resetexpiry", &first)]),
        (
            "another reference",
            [receipt("watchdog", "resetexpiry", "elsewhere")],
        ),
    ] {
        assert_eq!(
            crate::quota::reset_expiry_notices(&observation, &journal).len(),
            2,
            "{tag} is no receipt"
        );
    }
}

#[test]
fn banked_notice_identity_is_the_scope_and_utc_instant_not_the_label_or_offset() {
    let root = root();
    let utc = observe(&root, &[("cx", &dated(&[IN_24H_UTC]))]);
    let plus_one = observe(&root, &[("cx", &dated(&[IN_24H_PLUS_ONE]))]);
    let alias = observe(&root, &[("cy", &dated(&[IN_24H_UTC]))]);
    let reference = |observation: &crate::quota::Observation| {
        let found = crate::quota::reset_expiry_notices(observation, &[]);
        assert_eq!(found.len(), 1, "one reset inside the window");
        found[0].reference.clone()
    };
    let base = reference(&utc);
    assert_eq!(
        reference(&plus_one),
        base,
        "an equivalent offset is the same reset"
    );
    assert_eq!(
        reference(&alias),
        base,
        "another label on the same home is the same reset"
    );

    // Two labels on one config home, the same instant in two spellings: one
    // scope, one reset, one notice.
    let both = observe(
        &root,
        &[
            ("cx", &dated(&[IN_24H_UTC])),
            ("cy", &dated(&[IN_24H_PLUS_ONE])),
        ],
    );
    let found = crate::quota::reset_expiry_notices(&both, &[]);
    assert_eq!(found.len(), 1, "no duplicate within one observation");
    assert_eq!(found[0].reference, base);
    // And the receipt written under one label suppresses the other's.
    let journal = [receipt("watchdog", "resetexpiry", &base)];
    assert!(crate::quota::reset_expiry_notices(&alias, &journal).is_empty());
}

#[test]
fn banked_notice_at_equal_dated_counts_follows_the_earlier_first_difference() {
    // r7: two labels on one config home declare two resets each. The lists share
    // their first instant and differ at the second, so only a comparison of the
    // first DIFFERING item picks the earlier list. Both label orders must agree.
    let earlier = dated(&[IN_24H_UTC, IN_47H]);
    let later = dated(&[IN_24H_UTC, IN_48H]);
    for (tag, clients) in [
        (
            "earlier-first",
            [("cx", earlier.as_str()), ("cy", later.as_str())],
        ),
        (
            "later-first",
            [("cx", later.as_str()), ("cy", earlier.as_str())],
        ),
    ] {
        let observation = observe(&root(), &clients);
        let mut instants: Vec<i64> = crate::quota::reset_expiry_notices(&observation, &[])
            .iter()
            .map(|notice| notice.expiry)
            .collect();
        instants.sort_unstable();
        assert_eq!(
            instants,
            [NOW + 86_400, NOW + 169_200],
            "{tag}: the earlier list [+24 h, +47 h] wins, never [+24 h, +48 h]"
        );
    }
}
