//! #308 gap-1 acceptance spec, authored independently by effortdiag.
//! Oracle: brief-efforttie.md and lead rulings E1-E9; base 1008b4e7.

use super::*;
use crate::meta::{RecordedConfigHome, RecordedConfigHomeBase, RecordedWorkDir, RosterEntry};
use crate::quota::{Account, Group, Observation, Policy, Row};

const OBS: i64 = 1_700_000_000;
const NOW: i64 = OBS + 10;
const EARLY: i64 = OBS + 100;
const LATE: i64 = OBS + 200;
const VALID: &str = "v1 applied slot:spawned.0 base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:scout tool:codex launch:launch-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:solx client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700000100";

fn key(bucket: &str) -> WindowKey {
    WindowKey {
        bucket: bucket.into(),
        qualifier: None,
        minutes: Some(10_080),
    }
}

fn reading(percent: f64, reset: Option<i64>) -> Window {
    Window {
        key: key("weekly"),
        judged: percent,
        usable: true,
        observed_at: OBS,
        resets_at: reset,
    }
}

fn scope(windows: Vec<Window>) -> Scope {
    Scope {
        windows,
        spend_capped: false,
    }
}

fn crossed(scope: &Scope) -> Vec<Pressured> {
    match entry(scope, 80) {
        Entry::Crossed(windows) => windows,
        other => panic!("required logical-window crossing, got {other:?}"),
    }
}

fn one(scope: &Scope) -> Pressured {
    let windows = crossed(scope);
    assert_eq!(windows.len(), 1, "one logical key must be recorded once");
    windows.into_iter().next().expect("asserted one window")
}

fn pressed(reset: Option<i64>) -> Pressured {
    Pressured {
        key: key("weekly"),
        observed_at: OBS,
        resets_at: reset,
    }
}

#[test]
fn identical_ties_cross_once_even_with_more_than_eight_observations() {
    for count in [2, 9, 16] {
        let actual = one(&scope(vec![reading(80.0, Some(EARLY)); count]));
        assert_eq!(actual, pressed(Some(EARLY)), "{count} observations");
    }
}

#[test]
fn worst_tied_effective_pressure_crosses_independently_of_input_order() {
    for (left, right, crosses) in [(79.0, 81.0, true), (79.0, 80.0, true), (79.0, 79.0, false)] {
        for values in [[left, right], [right, left]] {
            let quota = scope(values.map(|p| reading(p, Some(EARLY))).to_vec());
            if crosses {
                assert_eq!(one(&quota), pressed(Some(EARLY)));
            } else {
                assert_eq!(entry(&quota, 80), Entry::Below);
            }
        }
    }
}

#[test]
fn mild_tied_sibling_contributes_later_or_unknown_reset() {
    for later in [Some(LATE), None] {
        let rows = [reading(81.0, Some(EARLY)), reading(79.0, later)];
        for quota in [
            scope(rows.to_vec()),
            scope(rows.into_iter().rev().collect()),
        ] {
            assert_eq!(one(&quota), pressed(later));
        }
    }
}

#[test]
fn later_tied_reset_survives_record_roundtrip_and_blocks_early_restore() {
    let quota = scope(vec![reading(80.0, Some(EARLY)), reading(81.0, Some(LATE))]);
    let mut record = Record::parse(VALID).expect("independent valid record");
    record.windows = crossed(&quota);
    let persisted = Record::parse(&record.encode()).expect("unchanged record grammar");
    assert_eq!(persisted.windows, vec![pressed(Some(LATE))]);
    let missing = scope(vec![]);
    for now in [EARLY - 1, EARLY, LATE - 1] {
        assert_eq!(
            recovery(&persisted.windows, 80, &missing, now),
            Recovery::Hold
        );
    }
    assert_eq!(
        recovery(&persisted.windows, 80, &missing, LATE),
        Recovery::Relieved
    );
    let mild = scope(vec![reading(74.0, Some(LATE)), reading(74.9, Some(LATE))]);
    assert_eq!(
        recovery(&persisted.windows, 80, &mild, NOW),
        Recovery::Relieved
    );
}

#[test]
fn unknown_tied_reset_never_gets_erased_by_known_reset() {
    for rows in [
        vec![reading(80.0, None), reading(80.0, Some(EARLY))],
        vec![reading(80.0, Some(EARLY)), reading(80.0, None)],
    ] {
        let recorded = crossed(&scope(rows));
        assert_eq!(recorded, vec![pressed(None)]);
        assert_eq!(
            recovery(&recorded, 80, &scope(vec![]), LATE),
            Recovery::Hold
        );
        let mixed = scope(vec![reading(74.0, None), reading(76.0, Some(LATE))]);
        assert_eq!(recovery(&recorded, 80, &mixed, NOW), Recovery::Hold);
        let mild = scope(vec![reading(74.0, None), reading(74.9, Some(LATE))]);
        assert_eq!(recovery(&recorded, 80, &mild, NOW), Recovery::Relieved);
    }
}

#[test]
fn recovery_requires_every_newest_tie_strictly_below_band() {
    let recorded = [pressed(None)];
    for (left, right, expected) in [
        (74.0, 76.0, Recovery::Hold),
        (74.0, 75.0, Recovery::Hold),
        (74.0, 74.9, Recovery::Relieved),
        (74.0, 80.0, Recovery::Hold),
        (74.0, f64::NAN, Recovery::Hold),
    ] {
        for values in [[left, right], [right, left]] {
            let quota = scope(values.map(|p| reading(p, None)).to_vec());
            assert_eq!(recovery(&recorded, 80, &quota, NOW), expected, "{values:?}");
        }
    }
}

#[test]
fn conservative_recorded_reset_override_and_hot_veto_remain() {
    let recorded = [pressed(Some(LATE))];
    let near = scope(vec![reading(76.0, Some(LATE)), reading(79.0, Some(LATE))]);
    assert_eq!(recovery(&recorded, 80, &near, LATE - 1), Recovery::Hold);
    assert_eq!(recovery(&recorded, 80, &near, LATE), Recovery::Relieved);
    let hot = scope(vec![reading(74.0, Some(LATE)), reading(80.0, Some(LATE))]);
    assert_eq!(recovery(&recorded, 80, &hot, LATE), Recovery::Hold);
}

#[test]
fn newest_usable_reading_supersedes_older_pressure_in_pure_entry_and_recovery() {
    let mut older = reading(90.0, Some(LATE));
    older.observed_at = OBS - 1;
    let newer = reading(74.0, Some(EARLY));
    for quota in [
        scope(vec![older.clone(), newer.clone()]),
        scope(vec![newer, older]),
    ] {
        assert_eq!(entry(&quota, 80), Entry::Below);
        assert_eq!(
            recovery(&[pressed(None)], 80, &quota, NOW),
            Recovery::Relieved
        );
    }
    let mut old_mild = reading(70.0, None);
    old_mild.observed_at = OBS - 1;
    let current_near = scope(vec![old_mild, reading(76.0, None)]);
    assert_eq!(
        recovery(&[pressed(None)], 80, &current_near, NOW),
        Recovery::Hold
    );
}

#[test]
fn superseded_reading_contributes_no_recorded_reset() {
    let mut older = reading(85.0, Some(LATE + 500));
    older.observed_at = OBS - 1;
    let current = reading(85.0, Some(EARLY));
    for rows in [vec![older.clone(), current.clone()], vec![current, older]] {
        assert_eq!(one(&scope(rows)), pressed(Some(EARLY)));
    }
}

#[test]
fn unusable_or_short_reading_cannot_supersede_usable_long_pressure() {
    let mut unusable = reading(0.0, None);
    unusable.usable = false;
    unusable.observed_at = OBS + 1;
    let mut short = reading(99.0, None);
    short.key.minutes = Some(300);
    let quota = scope(vec![reading(80.0, Some(EARLY)), unusable, short]);
    assert_eq!(one(&quota), pressed(Some(EARLY)));
}

#[test]
fn stale_relief_and_nan_never_supply_a_false_proof() {
    let mut older = reading(74.0, None);
    older.observed_at = OBS - 1;
    assert_eq!(
        recovery(&[pressed(None)], 80, &scope(vec![older]), NOW),
        Recovery::Hold
    );
    let mut later_recorded = pressed(None);
    later_recorded.observed_at = OBS + 1;
    let tied_mild = scope(vec![reading(74.0, None), reading(74.0, None)]);
    assert_eq!(
        recovery(&[later_recorded], 80, &tied_mild, NOW),
        Recovery::Hold
    );
    assert_eq!(
        one(&scope(vec![reading(f64::NAN, None), reading(85.0, None)])),
        pressed(None)
    );
}

#[test]
fn logical_key_limit_stays_eight_and_does_not_count_tied_observations() {
    for count in [8, 9] {
        let mut rows: Vec<_> = (0..count)
            .map(|i| {
                let mut row = reading(80.0, Some(EARLY));
                row.key.bucket = format!("w{i}");
                row
            })
            .collect();
        rows.push(rows[0].clone());
        let quota = scope(rows);
        if count == 8 {
            let windows = crossed(&quota);
            assert_eq!(windows.len(), 8);
            assert!(windows.windows(2).all(|pair| pair[0].key < pair[1].key));
        } else {
            assert_eq!(entry(&quota, 80), Entry::Unrecordable);
        }
    }
}

#[test]
fn all_window_key_dimensions_remain_distinct() {
    let mut qualified = reading(80.0, Some(EARLY));
    qualified.key.qualifier = Some("model".into());
    let mut daily = reading(80.0, Some(EARLY));
    daily.key.minutes = Some(1440);
    assert_eq!(
        crossed(&scope(vec![reading(80.0, Some(EARLY)), qualified, daily])).len(),
        3
    );
}

#[test]
fn merge_cannot_launder_invalid_tied_stamps_or_text() {
    for invalid_reset in [0, -1] {
        for invalid_pressure in [79.0, 81.0] {
            let rows = [
                reading(81.0, Some(LATE)),
                reading(invalid_pressure, Some(invalid_reset)),
            ];
            for quota in [
                scope(rows.to_vec()),
                scope(rows.into_iter().rev().collect()),
            ] {
                assert_eq!(entry(&quota, 80), Entry::Unrecordable);
            }
        }
    }
    for stamp in [0, -1] {
        let mut row = reading(81.0, Some(EARLY));
        row.observed_at = stamp;
        assert_eq!(
            entry(&scope(vec![row.clone(), row]), 80),
            Entry::Unrecordable
        );
    }
    for text in ["x".repeat(65), "bad\ntext".into(), String::new()] {
        for qualifier in [false, true] {
            let mut row = reading(81.0, Some(EARLY));
            if qualifier {
                row.key.qualifier = Some(text.clone());
            } else {
                row.key.bucket = text.clone();
            }
            assert_eq!(
                entry(&scope(vec![row.clone(), row]), 80),
                Entry::Unrecordable
            );
        }
    }
}

fn seat(binary: &str) -> RosterEntry {
    RosterEntry {
        slot: "spawned.0".into(),
        name: "scout".into(),
        profile: Some("fixture".into()),
        client: RecordedClient::Missing,
        harness_session: Some("22222222-2222-4222-8222-222222222222".into()),
        config_home: RecordedConfigHome::Path("/efforttie-fixture-account".into()),
        config_home_base: RecordedConfigHomeBase::Missing,
        binary: Some(binary.into()),
        work_dir: RecordedWorkDir::Missing,
    }
}

fn group(tool: ToolKind, source: &str, rows: Vec<Row>, account: Account) -> Group {
    Group {
        profiles: vec!["fixture".into()],
        tool,
        home: None,
        source: Some(source.into()),
        clients: vec![],
        rollout: None,
        owner: None,
        rows,
        hint: None,
        summary: None,
        policy: Policy::for_tests(None, account),
        notes: vec![],
        plan: None,
    }
}

fn observed(groups: Vec<Group>) -> Observation {
    Observation {
        groups,
        rendered: vec![],
        home: None,
        now: NOW,
    }
}

#[test]
fn codex_two_parsed_rollouts_join_the_same_account_and_cross_once() {
    let mut groups = Vec::new();
    for rollout in ["rollout-a", "rollout-b"] {
        let bytes = format!(
            "{{\"timestamp\":\"2023-11-14T22:13:20Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"rate_limits\":{{\"limit_id\":\"codex\",\"plan_type\":\"pro\",\"primary\":{{\"used_percent\":80,\"window_minutes\":10080,\"resets_at\":1700000200}},\"secondary\":null}},\"fixture_rollout\":\"{rollout}\"}}}}\n"
        );
        let parsed =
            crate::quota::codex::parse(bytes.as_bytes(), true, NOW).expect("synthetic rollout");
        assert_eq!(parsed.rows.len(), 1);
        let mut g = group(
            ToolKind::Codex,
            "/efforttie-fixture-account/sessions",
            parsed.rows,
            parsed.account,
        );
        g.rollout = Some(rollout.into());
        groups.push(g);
    }
    let quota = observed(groups);
    let own =
        own_scope(Some(&quota), &seat("codex"), None, NOW).expect("same recorded Codex account");
    let actual = one(&own);
    assert_eq!(actual.key.bucket, "codex");
    assert_eq!(actual.key.qualifier.as_deref(), Some("pro"));
    assert_eq!(actual.observed_at, OBS);
    assert_eq!(actual.resets_at, Some(LATE));
}

#[test]
fn claude_parsed_duplicate_limits_join_the_same_account_and_cross_once() {
    let bytes = br#"{"cachedUsageUtilization":{"fetchedAtMs":1700000000000,"utilization":{"limits":[{"kind":"weekly_all","percent":80,"resets_at":"2023-11-14T22:16:40Z"},{"kind":"weekly_all","percent":80,"resets_at":"2023-11-14T22:16:40Z"}]}}}"#;
    let parsed = crate::quota::claude::parse(bytes, NOW).expect("synthetic Claude cache");
    let snapshot = parsed.snapshot.expect("quota cache present");
    assert_eq!(snapshot.rows.len(), 2, "producer emits both tied limits");
    let quota = observed(vec![group(
        ToolKind::Claude,
        "/efforttie-fixture-account/.claude.json",
        snapshot.rows,
        Account::default(),
    )]);
    let own =
        own_scope(Some(&quota), &seat("claude"), None, NOW).expect("same recorded Claude account");
    let actual = one(&own);
    assert_eq!(actual.key, key("weekly_all"));
    assert_eq!(actual.observed_at, OBS);
    assert_eq!(actual.resets_at, Some(LATE));
}

#[test]
fn tied_crossing_opens_then_authorizes_one_window_step_down() {
    let quota = scope(vec![reading(80.0, Some(EARLY)); 2]);
    let armed = Fold::Armed;
    let absent = Stored::Absent;
    let twin = Twin {
        baseline: Level::Xhigh,
        target: Level::High,
        aliases: vec!["solh".into()],
    };
    let configured = Ok(twin.clone());
    let mut input = Inputs {
        at: 80,
        new_attempts: true,
        scope: Some(&quota),
        limited: false,
        fold: &armed,
        stored: &absent,
        guard: Some(Ok(())),
        twin: &configured,
        live: Some(Level::Xhigh),
        now: NOW,
    };
    assert_eq!(decide(&input), Decision::Open { threshold: 80 });
    let opened = Fold::Open(Episode {
        key: OBS,
        threshold: 80,
        down: Leg::Unspent,
        up: Leg::Unspent,
        named: vec![],
        yielded: false,
    });
    input.fold = &opened;
    assert_eq!(
        decide(&input),
        Decision::Down {
            twin,
            windows: vec![pressed(Some(EARLY))]
        }
    );
}
