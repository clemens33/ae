//! Frozen acceptance spec (slice limitwake): the binding reset of a
//! seat's OWN spent scope. Oracle: brief `.local/brief-limitwake.md` —
//! the epoch comes from a held quota reading, never from zone text;
//! unknown reset means no wake and no wait.
//!
//! Installed byte-identical at `src/autoreseat/leg_limitwake_spec.rs`,
//! wired from `src/autoreseat/leg.rs` via
//! `#[cfg(test)] #[path = "leg_limitwake_spec.rs"] mod leg_limitwake_spec;`.

use super::own_reset;
use crate::autoreseat::LimitReset;
use crate::meta::{
    RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RecordedWorkDir, RosterEntry,
};
use crate::quota::{Account, Group, Observation, Policy, Row, Status};
use crate::tool::ToolKind;

const NOW: i64 = 1_700_000_010;
const SOURCE: &str = "/limitwake-spec-account/.claude.json";

fn seat() -> RosterEntry {
    RosterEntry {
        slot: "spawned.0".into(),
        name: "scout".into(),
        profile: Some("opusx".into()),
        client: RecordedClient::Missing,
        harness_session: None,
        config_home: RecordedConfigHome::Path("/limitwake-spec-account".into()),
        config_home_base: RecordedConfigHomeBase::Missing,
        binary: Some("claude".into()),
        work_dir: RecordedWorkDir::Missing,
    }
}

fn row(
    bucket: &str,
    qualifier: Option<&str>,
    minutes: Option<u32>,
    percent: &str,
    resets_at: Option<i64>,
    observed_at: i64,
) -> Row {
    Row {
        bucket: bucket.into(),
        qualifier: qualifier.map(str::to_owned),
        window_minutes: minutes,
        used_percent: Some(percent.into()),
        resets_at,
        observed_at: Some(observed_at),
        status: Status::Fresh,
    }
}

fn group(source: &str, rows: Vec<Row>, policy: Policy) -> Group {
    Group {
        profiles: vec!["opusx".into()],
        tool: ToolKind::Claude,
        home: None,
        source: Some(source.into()),
        clients: vec![],
        rollout: None,
        owner: None,
        rows,
        hint: None,
        summary: None,
        policy,
        notes: vec![],
        plan: None,
    }
}

fn observation(groups: Vec<Group>) -> Observation {
    Observation {
        groups,
        rendered: vec![],
        home: None,
        now: NOW,
    }
}

fn one(the_row: Row) -> Observation {
    observation(vec![group(SOURCE, vec![the_row], Policy::default())])
}

#[test]
fn a_spent_short_window_yields_its_reset_proven_short() {
    let observed = one(row(
        "session",
        None,
        Some(300),
        "100",
        Some(NOW + 1800),
        NOW,
    ));
    assert_eq!(
        own_reset(Some(&observed), &seat(), None, NOW),
        Some(LimitReset {
            at: NOW + 1800,
            short: true
        })
    );
    // The boundary of proven-short is the one day line: 1439 states a
    // short window, 1440 states a long one.
    let under = one(row("session", None, Some(1439), "100", Some(NOW + 60), NOW));
    assert_eq!(
        own_reset(Some(&under), &seat(), None, NOW),
        Some(LimitReset {
            at: NOW + 60,
            short: true
        })
    );
    let daily = one(row("daily", None, Some(1440), "100", Some(NOW + 60), NOW));
    assert_eq!(
        own_reset(Some(&daily), &seat(), None, NOW),
        Some(LimitReset {
            at: NOW + 60,
            short: false
        })
    );
}

#[test]
fn the_binding_reset_is_the_latest_and_any_long_window_makes_it_not_short() {
    let observed = observation(vec![group(
        SOURCE,
        vec![
            row("session", None, Some(300), "100", Some(NOW + 1800), NOW),
            row(
                "weekly",
                None,
                Some(10_080),
                "100",
                Some(NOW + 100_000),
                NOW,
            ),
        ],
        Policy::default(),
    )]);
    assert_eq!(
        own_reset(Some(&observed), &seat(), None, NOW),
        Some(LimitReset {
            at: NOW + 100_000,
            short: false
        }),
        "waking at the 5h reset while the weekly still binds would re-hit at once"
    );
}

#[test]
fn an_unstated_length_is_never_proven_short() {
    let observed = one(row("session", None, None, "100", Some(NOW + 1800), NOW));
    assert_eq!(
        own_reset(Some(&observed), &seat(), None, NOW),
        Some(LimitReset {
            at: NOW + 1800,
            short: false
        }),
        "the reset is known so a wake may use it; the length is not, so no wait"
    );
}

#[test]
fn a_full_bound_window_with_no_reset_makes_the_reset_unknown() {
    let alone = one(row("session", None, Some(300), "100", None, NOW));
    assert_eq!(own_reset(Some(&alone), &seat(), None, NOW), None);
    let mixed = observation(vec![group(
        SOURCE,
        vec![
            row("session", None, Some(300), "100", Some(NOW + 1800), NOW),
            row("weekly", None, Some(10_080), "100", None, NOW),
        ],
        Policy::default(),
    )]);
    assert_eq!(
        own_reset(Some(&mixed), &seat(), None, NOW),
        None,
        "one binding window's reset is unstated, so THE reset is unknown"
    );
}

#[test]
fn a_spend_cap_has_no_reset_that_lifts_it() {
    let capped = Policy::for_tests(
        None,
        Account {
            spend_control_reached: Some(true),
            spend_observed_at: Some(NOW),
            ..Account::default()
        },
    );
    let observed = observation(vec![group(
        SOURCE,
        vec![row(
            "session",
            None,
            Some(300),
            "100",
            Some(NOW + 1800),
            NOW,
        )],
        capped,
    )]);
    assert_eq!(own_reset(Some(&observed), &seat(), None, NOW), None);
}

#[test]
fn only_a_fresh_full_unanswered_bound_window_binds() {
    // Not full.
    let healthy = one(row("session", None, Some(300), "99", Some(NOW + 1800), NOW));
    assert_eq!(own_reset(Some(&healthy), &seat(), None, NOW), None);
    // Stale: a reading 1000 s old proves nothing (fresh is 900 s).
    let stale = one(row(
        "session",
        None,
        Some(300),
        "100",
        Some(NOW + 1800),
        NOW - 1000,
    ));
    assert_eq!(own_reset(Some(&stale), &seat(), None, NOW), None);
    // Scoped to another model family: not this seat's binding window.
    let foreign = one(row(
        "session",
        Some("sonnet"),
        Some(300),
        "100",
        Some(NOW + 1800),
        NOW,
    ));
    assert_eq!(own_reset(Some(&foreign), &seat(), Some("opus"), NOW), None);
    assert_eq!(
        own_reset(Some(&foreign), &seat(), Some("sonnet"), NOW),
        Some(LimitReset {
            at: NOW + 1800,
            short: true
        })
    );
    // Answered: a healthy reading of the same window as new as the full
    // one takes its proof away; an older healthy one does not.
    let answered = |healthy_at: i64| {
        observation(vec![group(
            SOURCE,
            vec![
                row(
                    "session",
                    None,
                    Some(300),
                    "100",
                    Some(NOW + 1800),
                    NOW - 60,
                ),
                row(
                    "session",
                    None,
                    Some(300),
                    "50",
                    Some(NOW + 1800),
                    healthy_at,
                ),
            ],
            Policy::default(),
        )])
    };
    assert_eq!(
        own_reset(Some(&answered(NOW)), &seat(), None, NOW),
        None,
        "the newest reading of the window is healthy"
    );
    assert_eq!(
        own_reset(Some(&answered(NOW - 120)), &seat(), None, NOW),
        Some(LimitReset {
            at: NOW + 1800,
            short: true
        }),
        "an older healthy reading cannot answer a newer full one"
    );
}

#[test]
fn nothing_is_read_without_a_proven_identity_an_observation_or_the_seats_own_scope() {
    let observed = one(row(
        "session",
        None,
        Some(300),
        "100",
        Some(NOW + 1800),
        NOW,
    ));
    let mut unproven = seat();
    unproven.config_home = RecordedConfigHome::Missing;
    assert_eq!(own_reset(Some(&observed), &unproven, None, NOW), None);
    assert_eq!(own_reset(None, &seat(), None, NOW), None);
    let foreign_account = observation(vec![group(
        "/limitwake-spec-other/.claude.json",
        vec![row(
            "session",
            None,
            Some(300),
            "100",
            Some(NOW + 1800),
            NOW,
        )],
        Policy::default(),
    )]);
    assert_eq!(
        own_reset(Some(&foreign_account), &seat(), None, NOW),
        None,
        "another account's reset is not this seat's"
    );
}
