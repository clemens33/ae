//! Frozen phase-1 effort contract. Oracles: frozen-c98ebc3f + L8 amendment.
//! All values and journal traces are independently authored; no live actuator.

use std::path::Path;

use ae::config::parse_identity;
use ae::effort::{
    Damage, Decision, Direction, Entry, Episode, Fold, Foreign, Hold, Inputs, Leg, Level,
    Pressured, Record, Recovery, Scope, SeatNow, State, Stored, Transition, Twin, TwinRefusal,
    decide, entry, fold, guard, recovery, relieved, stored, twin_in,
};
use ae::events::{AlertMeaning, Event, SessionKey};
use ae::launch_cmd::{effort_flag, effort_skeleton, replace_effort_flag};
use ae::meta::RecordedClient;
use ae::quota::WindowKey;
use ae::tool::ToolKind;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const CONV: &str = "22222222-2222-4222-8222-222222222222";
const OTHER: &str = "33333333-3333-4333-8333-333333333333";
const EP: i64 = 1_700_000_000;
const NOW: i64 = EP + 10;
const VALID: &str = "v1 applied slot:spawned.0 base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:scout tool:codex launch:launch-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:solx client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700000100";

fn key(bucket: &str, minutes: Option<u32>) -> WindowKey {
    WindowKey {
        bucket: bucket.into(),
        qualifier: None,
        minutes,
    }
}

fn window(bucket: &str, percent: f64, minutes: Option<u32>) -> ae::effort::Window {
    ae::effort::Window {
        key: key(bucket, minutes),
        judged: percent,
        usable: true,
        observed_at: EP,
        resets_at: Some(EP + 100),
    }
}

fn scope(percent: f64) -> Scope {
    Scope {
        windows: vec![window("weekly", percent, Some(10_080))],
        spend_capped: false,
    }
}

fn pressured(bucket: &str, reset: Option<i64>) -> Pressured {
    Pressured {
        key: key(bucket, Some(10_080)),
        observed_at: EP,
        resets_at: reset,
    }
}

fn record() -> Record {
    Record {
        state: State::Applied,
        slot: "spawned.0".into(),
        base: Level::Xhigh,
        low: Level::High,
        threshold: 80,
        episode: EP,
        session_id: SID.into(),
        agent: "scout".into(),
        tool: ToolKind::Codex,
        launch: "launch-1".into(),
        pid: 42,
        conversation: CONV.into(),
        profile: "solx".into(),
        client: None,
        model: "gpt-6.1-sol".into(),
        windows: vec![pressured("weekly", Some(EP + 100))],
    }
}

fn down() -> Transition {
    Transition {
        direction: Direction::Down,
        from: Level::Xhigh,
        to: Level::High,
    }
}

fn up() -> Transition {
    Transition {
        direction: Direction::Up,
        from: Level::High,
        to: Level::Xhigh,
    }
}

fn episode() -> Episode {
    Episode {
        key: EP,
        threshold: 80,
        down: Leg::Unspent,
        up: Leg::Unspent,
        named: vec![],
        yielded: false,
    }
}

fn twin() -> Twin {
    Twin {
        baseline: Level::Xhigh,
        target: Level::High,
        aliases: vec!["solh".into()],
    }
}

#[cfg(test)]
fn event(action: &str, reference: &str, summary: &str) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"{action}","target":"scout","target_slot":"spawned.0","target_session":"before-rename","target_session_id":"{SID}","ref":"{reference}","summary":"{summary}"}}"#
    )).expect("independent valid journal fixture")
}

fn opened() -> Vec<Event> {
    vec![event("effort-episode", "1700000000", "threshold 80")]
}

fn applied() -> Vec<Event> {
    let mut events = opened();
    events.push(event("effort-reserved", "1700000000-down", "xhigh->high"));
    events.push(event("effort-applied", "1700000000-down", "xhigh->high"));
    events
}

fn folded(events: &[Event]) -> Fold {
    fold(
        events,
        SessionKey::new("after-rename", SID),
        "spawned.0",
        "scout",
    )
}

fn inputs<'a>(
    scope: &'a Scope,
    fold: &'a Fold,
    stored: &'a Stored,
    twin: &'a Result<Twin, TwinRefusal>,
) -> Inputs<'a> {
    Inputs {
        at: 80,
        new_attempts: true,
        scope: Some(scope),
        limited: false,
        fold,
        stored,
        guard: Some(Ok(())),
        twin,
        live: Some(Level::Xhigh),
        now: NOW,
    }
}

#[test]
fn effort_levels_exact_vocabulary_and_one_step_floor() {
    for (word, level, lower) in [
        ("minimal", Level::Minimal, None),
        ("low", Level::Low, None),
        ("medium", Level::Medium, None),
        ("high", Level::High, None),
        ("xhigh", Level::Xhigh, Some(Level::High)),
        ("max", Level::Max, Some(Level::Xhigh)),
        ("ultra", Level::Ultra, None),
    ] {
        assert_eq!(Level::parse(word), Some(level), "literal vocabulary {word}");
        assert_eq!(level.word(), word);
        assert_eq!(level.step_down(), lower, "exact one-tier contract {word}");
    }
    for word in [
        "",
        "High",
        "extra high",
        " xhigh",
        "max ",
        "none",
        "xhigh\n",
    ] {
        assert_eq!(Level::parse(word), None, "noncanonical {word:?}");
    }
}

#[test]
fn effort_transitions_are_exact_inverse_pairs() {
    assert_eq!(Transition::down(Level::Xhigh), Some(down()));
    assert_eq!(Transition::up(Level::Xhigh), Some(up()));
    assert_eq!(
        Transition::down(Level::Max),
        Some(Transition {
            direction: Direction::Down,
            from: Level::Max,
            to: Level::Xhigh
        })
    );
    assert_eq!(
        Transition::up(Level::Max),
        Some(Transition {
            direction: Direction::Up,
            from: Level::Xhigh,
            to: Level::Max
        })
    );
    for level in [
        Level::Minimal,
        Level::Low,
        Level::Medium,
        Level::High,
        Level::Ultra,
    ] {
        assert_eq!(Transition::down(level), None);
        assert_eq!(Transition::up(level), None);
    }
}

#[test]
fn effort_flag_reads_only_recognized_single_occurrences() {
    for (tool, command, level) in [
        (
            ToolKind::Claude,
            "claude --model opus --effort xhigh",
            Level::Xhigh,
        ),
        (
            ToolKind::Claude,
            "claude --model opus --effort=max",
            Level::Max,
        ),
        (
            ToolKind::Claude,
            "env -u OTHER /opt/claude --effort 'high'",
            Level::High,
        ),
        (
            ToolKind::Codex,
            "codex -m gpt -c model_reasoning_effort=xhigh",
            Level::Xhigh,
        ),
        (
            ToolKind::Codex,
            "codex --config model_reasoning_effort=max -m gpt",
            Level::Max,
        ),
        (
            ToolKind::Codex,
            "CODEX_HOME=/scratch/a codex --config=model_reasoning_effort=high -m gpt",
            Level::High,
        ),
        (
            ToolKind::Codex,
            "codex -c other=true -c model_reasoning_effort=ultra",
            Level::Ultra,
        ),
    ] {
        assert_eq!(effort_flag(command, tool), Ok(level), "{command}");
    }
}

#[test]
fn effort_flag_refuses_ambiguous_missing_unknown_environment_and_shell_shapes() {
    for (tool, command, reason) in [
        (ToolKind::Claude, "claude --model opus", "no-effort-flag"),
        (ToolKind::Claude, "claude --effort", "effort-missing-value"),
        (ToolKind::Claude, "claude --effort=", "effort-missing-value"),
        (
            ToolKind::Claude,
            "claude --effort --model opus",
            "effort-missing-value",
        ),
        (
            ToolKind::Claude,
            "claude --effort high --effort=max",
            "ambiguous-effort",
        ),
        (
            ToolKind::Claude,
            "claude --effort high --effort-other=max",
            "ambiguous-effort",
        ),
        (ToolKind::Claude, "claude --effort ultra", "unknown-effort"),
        (
            ToolKind::Claude,
            "CLAUDE_CODE_EFFORT_LEVEL=max claude --effort high",
            "env-effort",
        ),
        (
            ToolKind::Claude,
            "env -u CLAUDE_CODE_EFFORT_LEVEL claude --effort high",
            "env-effort",
        ),
        (
            ToolKind::Claude,
            "env CLAUDE_CODE_EFFORT_LEVEL=max claude --effort high",
            "env-effort",
        ),
        (
            ToolKind::Claude,
            "claude --effort high; echo changed",
            "not-simple",
        ),
        (
            ToolKind::Claude,
            "claude --effort $(echo high)",
            "not-simple",
        ),
        (ToolKind::Codex, "codex -c other=true", "no-effort-flag"),
        (
            ToolKind::Codex,
            "codex -c model_reasoning_effort=",
            "effort-missing-value",
        ),
        (
            ToolKind::Codex,
            "codex -c model_reasoning_effort=High",
            "unknown-effort",
        ),
        (
            ToolKind::Codex,
            "codex -c 'model_reasoning_effort=\"xhigh\"'",
            "unknown-effort",
        ),
        (
            ToolKind::Codex,
            "codex -cmodel_reasoning_effort=high",
            "ambiguous-effort",
        ),
        (
            ToolKind::Codex,
            "codex -c=model_reasoning_effort=high",
            "ambiguous-effort",
        ),
        (
            ToolKind::Codex,
            "codex --config profiles.x.model_reasoning_effort=high",
            "ambiguous-effort",
        ),
        (
            ToolKind::Codex,
            "codex -c 'model_reasoning_effort = high'",
            "ambiguous-effort",
        ),
        (
            ToolKind::Codex,
            "codex -c model_reasoning_effort=high --config=model_reasoning_effort=max",
            "ambiguous-effort",
        ),
        (ToolKind::Muse, "muse --reasoning-effort max", "unsupported"),
        (ToolKind::Unknown, "other --effort max", "unsupported"),
    ] {
        assert_eq!(
            effort_flag(command, tool).map_err(|e| e.word()),
            Err(reason),
            "{command}"
        );
    }
}

#[test]
fn effort_rewriter_preserves_account_model_permissions_and_flag_spelling() {
    for (tool, before, after) in [
        (
            ToolKind::Claude,
            "CLAUDE_CONFIG_DIR='/scratch/account a' /opt/claude --model opus --effort xhigh --permission-mode bypassPermissions",
            "CLAUDE_CONFIG_DIR='/scratch/account a' /opt/claude --model opus --effort high --permission-mode bypassPermissions",
        ),
        (
            ToolKind::Claude,
            "claude --effort=max --model opus",
            "claude --effort=xhigh --model opus",
        ),
        (
            ToolKind::Codex,
            "codex -m gpt --yolo -c model_reasoning_effort=xhigh -c other=true",
            "codex -m gpt --yolo -c model_reasoning_effort=high -c other=true",
        ),
        (
            ToolKind::Codex,
            "codex --config=model_reasoning_effort=max -m gpt",
            "codex --config=model_reasoning_effort=xhigh -m gpt",
        ),
    ] {
        let target = if before.contains("=max") {
            Level::Xhigh
        } else {
            Level::High
        };
        assert_eq!(replace_effort_flag(before, tool, target), Ok(after.into()));
    }
    assert!(replace_effort_flag("claude --model opus", ToolKind::Claude, Level::High).is_err());
    assert!(
        replace_effort_flag(
            "claude --effort high --effort=max",
            ToolKind::Claude,
            Level::High
        )
        .is_err()
    );
}

#[test]
fn effort_skeleton_collapses_only_effort_value_and_split_joined_forms() {
    let actual = effort_skeleton(
        "CODEX_HOME=/scratch/account codex -m gpt --yolo -c model_reasoning_effort=xhigh -c other=true",
        ToolKind::Codex,
    );
    let expected = [
        "CODEX_HOME=/scratch/account",
        "codex",
        "-m",
        "gpt",
        "--yolo",
        "-c model_reasoning_effort",
        "-c",
        "other=true",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(actual, Ok((expected, Level::Xhigh)));
    let split = effort_skeleton("claude --model opus --effort xhigh", ToolKind::Claude);
    let joined = effort_skeleton("claude --model opus --effort=high", ToolKind::Claude);
    assert_eq!(split.map(|(s, _)| s), joined.map(|(s, _)| s));
}

#[test]
fn effort_twins_use_exact_tier_and_collapse_target_aliases() {
    let cfg = parse_identity("[profiles]\nbase=codex -m gpt -c model_reasoning_effort=max\nx1=codex -m gpt -c model_reasoning_effort=xhigh\nx2=codex -m gpt -c model_reasoning_effort=xhigh\nh=codex -m gpt -c model_reasoning_effort=high\n").expect("config fixture");
    assert_eq!(
        twin_in(
            &cfg,
            None,
            "base",
            &RecordedClient::Missing,
            ToolKind::Codex
        ),
        Ok(Twin {
            baseline: Level::Max,
            target: Level::Xhigh,
            aliases: vec!["x1".into(), "x2".into()]
        })
    );
    assert_eq!(
        twin_in(&cfg, None, "x1", &RecordedClient::Missing, ToolKind::Codex),
        Ok(Twin {
            baseline: Level::Xhigh,
            target: Level::High,
            aliases: vec!["h".into()]
        })
    );
}

#[test]
fn effort_twins_never_skip_missing_xhigh_or_lower_the_floor() {
    let cfg = parse_identity("[profiles]\nbase=codex -m gpt -c model_reasoning_effort=max\nh=codex -m gpt -c model_reasoning_effort=high\nultra=codex -m gpt -c model_reasoning_effort=ultra\nlow=codex -m gpt -c model_reasoning_effort=low\n").expect("config fixture");
    for (name, reason) in [
        ("base", "no-twin"),
        ("h", "floor"),
        ("low", "floor"),
        ("ultra", "unsupported-level"),
    ] {
        assert_eq!(
            twin_in(&cfg, None, name, &RecordedClient::Missing, ToolKind::Codex)
                .map_err(|e| e.word()),
            Err(reason)
        );
    }
}

#[test]
fn effort_twins_refuse_any_other_command_change() {
    let baseline = "CODEX_HOME=/scratch/a /opt/codex -m gpt --yolo -c model_reasoning_effort=xhigh";
    for candidate in [
        "CODEX_HOME=/scratch/b /opt/codex -m gpt --yolo -c model_reasoning_effort=high",
        "CODEX_HOME=/scratch/a codex -m gpt --yolo -c model_reasoning_effort=high",
        "CODEX_HOME=/scratch/a /opt/codex -m other --yolo -c model_reasoning_effort=high",
        "CODEX_HOME=/scratch/a /opt/codex -m gpt -c model_reasoning_effort=high",
        "CODEX_HOME=/scratch/a /opt/codex --yolo -m gpt -c model_reasoning_effort=high",
        "CODEX_HOME=/scratch/a /opt/codex -m gpt --yolo -c model_reasoning_effort=high -c other=true",
    ] {
        let cfg = parse_identity(&format!("[profiles]\nbase={baseline}\nother={candidate}\n"))
            .expect("config fixture");
        assert_eq!(
            twin_in(
                &cfg,
                None,
                "base",
                &RecordedClient::Missing,
                ToolKind::Codex
            )
            .map_err(|e| e.word()),
            Err("no-twin"),
            "{candidate}"
        );
    }
}

#[test]
fn effort_twins_resolve_same_recorded_client_override_for_both_profiles() {
    let cfg = parse_identity("[clients]\npersonal=/opt/claude config_home=/scratch/personal\nwork=/opt/claude config_home=/scratch/work\n[profiles]\nbase=personal --model opus --effort xhigh\nlower=work --model opus --effort high\n").expect("client config fixture");
    assert_eq!(
        twin_in(
            &cfg,
            Some(Path::new("/scratch/home")),
            "base",
            &RecordedClient::Label("work".into()),
            ToolKind::Claude
        ),
        Ok(Twin {
            baseline: Level::Xhigh,
            target: Level::High,
            aliases: vec!["lower".into()]
        })
    );
    assert_eq!(
        twin_in(
            &cfg,
            None,
            "base",
            &RecordedClient::Missing,
            ToolKind::Claude
        )
        .map_err(|e| e.word()),
        Err("no-twin")
    );
    assert_eq!(
        twin_in(
            &cfg,
            None,
            "base",
            &RecordedClient::Invalid,
            ToolKind::Claude
        )
        .map_err(|e| e.word()),
        Err("unresolved")
    );
}

#[test]
fn effort_twins_require_single_model_and_known_harness() {
    for command in [
        "claude --effort xhigh",
        "claude --model opus --model other --effort xhigh",
    ] {
        let cfg = parse_identity(&format!("[profiles]\nbase={command}\n")).expect("config fixture");
        assert_eq!(
            twin_in(
                &cfg,
                None,
                "base",
                &RecordedClient::Missing,
                ToolKind::Claude
            )
            .map_err(|e| e.word()),
            Err("model-unproven")
        );
    }
    let cfg = parse_identity("[profiles]\nbase=muse --model muse --reasoning-effort max\n")
        .expect("config fixture");
    assert_eq!(
        twin_in(&cfg, None, "base", &RecordedClient::Missing, ToolKind::Muse).map_err(|e| e.word()),
        Err("unsupported")
    );
}

#[test]
fn effort_entry_boundary_and_l8_short_window_exclusion() {
    assert_eq!(entry(&scope(79.9), 80), Entry::Below);
    assert_eq!(
        entry(&scope(80.0), 80),
        Entry::Crossed(vec![pressured("weekly", Some(EP + 100))])
    );
    let mut s = scope(40.0);
    s.windows.push(window("five_hour", 95.0, Some(300)));
    s.windows.push(window("unknown_length", 100.0, None));
    assert_eq!(
        entry(&s, 80),
        Entry::Below,
        "weekly40 outranks short95 and unknown100"
    );
    s.windows[0].judged = 80.0;
    s.windows[1].judged = 10.0;
    assert_eq!(
        entry(&s, 80),
        Entry::Crossed(vec![pressured("weekly", Some(EP + 100))])
    );
}

#[test]
fn effort_long_window_proof_is_inclusive_at_one_day() {
    for (minutes, usable, long) in [
        (None, true, false),
        (Some(1439), true, false),
        (Some(1440), true, true),
        (Some(10_080), true, true),
        (Some(10_080), false, false),
    ] {
        let mut w = window("w", 99.0, minutes);
        w.usable = usable;
        assert_eq!(w.long(), long, "length {minutes:?} usable {usable}");
    }
}

#[test]
fn effort_no_usable_long_reading_is_unread_not_below() {
    let mut s = Scope {
        windows: vec![
            window("short", 95.0, Some(300)),
            window("unknown", 95.0, None),
        ],
        spend_capped: false,
    };
    assert_eq!(entry(&s, 80), Entry::Unread);
    assert_eq!(relieved(&s, 80), None);
    s.windows.push(window("weekly", 20.0, Some(10_080)));
    s.windows[2].usable = false;
    assert_eq!(entry(&s, 80), Entry::Unread);
    assert_eq!(
        relieved(&s, 80),
        None,
        "expired or malformed weekly rows prove no relief"
    );
}

#[test]
fn effort_entry_saves_every_long_pressured_key_not_only_worst() {
    let mut s = scope(81.0);
    s.windows.push(window("daily", 82.0, Some(1440)));
    s.windows.push(window("short", 99.0, Some(300)));
    let mut daily = pressured("daily", Some(EP + 100));
    daily.key.minutes = Some(1440);
    assert_eq!(
        entry(&s, 80),
        Entry::Crossed(vec![daily, pressured("weekly", Some(EP + 100))])
    );
}

#[test]
fn effort_entry_refuses_overflow_and_unencodable_pressured_set() {
    let s = Scope {
        windows: (0..9)
            .map(|i| window(&format!("w{i}"), 90.0, Some(10_080)))
            .collect(),
        spend_capped: false,
    };
    assert_eq!(entry(&s, 80), Entry::Unrecordable);
    let s = Scope {
        windows: vec![window(&"x".repeat(65), 90.0, Some(10_080))],
        spend_capped: false,
    };
    assert_eq!(entry(&s, 80), Entry::Unrecordable);
}

#[test]
fn effort_recovery_exact_hysteresis_and_older_observation_hold() {
    let r = vec![pressured("weekly", None)];
    assert_eq!(recovery(&r, 80, &scope(75.0), NOW), Recovery::Hold);
    assert_eq!(recovery(&r, 80, &scope(74.9), NOW), Recovery::Relieved);
    let mut s = scope(0.0);
    s.windows[0].observed_at = EP - 1;
    assert_eq!(
        recovery(&r, 80, &s, NOW),
        Recovery::Hold,
        "older sample cannot clear recorded pressure"
    );
    s.windows[0].observed_at = EP;
    assert_eq!(
        recovery(&r, 80, &s, NOW),
        Recovery::Relieved,
        "same raw stamp may carry changed policy"
    );
}

#[test]
fn effort_recovery_requires_all_recorded_windows_and_only_threshold_veto() {
    let r = vec![
        pressured("weekly", Some(EP + 100)),
        pressured("other_week", Some(EP + 100)),
    ];
    let mut s = scope(74.9);
    s.windows.push(window("other_week", 82.0, Some(10_080)));
    assert_eq!(recovery(&r, 80, &s, NOW), Recovery::Hold);
    s.windows[1].judged = 74.9;
    s.windows.push(window("new_week", 76.0, Some(10_080)));
    assert_eq!(
        recovery(&r, 80, &s, NOW),
        Recovery::Relieved,
        "previously unpressured76 does not veto"
    );
    s.windows[2].judged = 80.0;
    assert_eq!(recovery(&r, 80, &s, NOW), Recovery::Hold, "new80 vetoes");
    s.windows[2].key.minutes = Some(300);
    assert_eq!(
        recovery(&r, 80, &s, NOW),
        Recovery::Relieved,
        "short80 cannot veto weekly relief"
    );
}

#[test]
fn effort_applied_recovery_keeps_reset_only_clear_with_no_current_long_row() {
    let r = vec![pressured("weekly", Some(NOW))];
    let mut s = Scope {
        windows: vec![],
        spend_capped: false,
    };
    assert_eq!(recovery(&r, 80, &s, NOW - 1), Recovery::Hold);
    assert_eq!(
        recovery(&r, 80, &s, NOW),
        Recovery::Relieved,
        "L8 precedence preserves B3 reset-only"
    );
    assert_eq!(entry(&s, 80), Entry::Unread);
    assert_eq!(
        relieved(&s, 80),
        None,
        "reset-only never re-arms a non-applied episode"
    );
    s.windows.push(window("weekly", 80.0, Some(10_080)));
    s.windows[0].observed_at = NOW;
    assert_eq!(
        recovery(&r, 80, &s, NOW),
        Recovery::Hold,
        "fresh post-reset80 outranks recorded reset"
    );
    s.windows[0].judged = 76.0;
    assert_eq!(recovery(&r, 80, &s, NOW), Recovery::Relieved);
}

#[test]
fn effort_weekly_pressure_survives_other_window_reset() {
    let r = vec![
        pressured("weekly", Some(EP + 100)),
        pressured("other_week", Some(NOW)),
    ];
    assert_eq!(recovery(&r, 80, &scope(82.0), NOW), Recovery::Hold);
    assert_eq!(recovery(&r, 80, &scope(74.9), NOW), Recovery::Relieved);
}

#[test]
fn effort_missing_relief_without_known_reset_holds() {
    let s = Scope {
        windows: vec![],
        spend_capped: false,
    };
    assert_eq!(
        recovery(&[pressured("weekly", None)], 80, &s, NOW),
        Recovery::Hold
    );
    assert_eq!(
        recovery(&[pressured("weekly", Some(EP + 100))], 80, &s, NOW),
        Recovery::Hold
    );
}

#[test]
fn effort_spend_cap_precedes_entry_recovery_reset_and_length() {
    let s = Scope {
        windows: vec![window("short", 0.0, Some(300))],
        spend_capped: true,
    };
    assert_eq!(entry(&s, 80), Entry::SpendCapped);
    assert_eq!(
        recovery(&[pressured("weekly", Some(NOW - 1))], 80, &s, NOW),
        Recovery::SpendCapped
    );
    assert_eq!(
        relieved(&s, 80),
        Some(false),
        "cap never proves room even on a short-only scope"
    );
}

#[test]
fn effort_non_applied_rearm_uses_only_long_room_hysteresis() {
    assert_eq!(relieved(&scope(79.0), 80), Some(false));
    assert_eq!(relieved(&scope(75.0), 80), Some(false));
    assert_eq!(relieved(&scope(74.9), 80), Some(true));
    let mut s = scope(74.9);
    s.windows.push(window("short", 100.0, Some(300)));
    assert_eq!(relieved(&s, 80), Some(true));
    s.windows.push(window("other_week", 76.0, Some(10_080)));
    assert_eq!(
        relieved(&s, 80),
        Some(false),
        "room re-arm differs from B3 applied recovery"
    );
}

#[test]
fn effort_record_literal_pending_and_applied_states_are_canonical() {
    assert_eq!(Record::parse(VALID), Ok(record()), "literal meta contract");
    assert_eq!(record().encode(), VALID);
    assert_eq!(record().pending(), None);
    for (word, state, transition) in [
        ("pending-down", State::PendingDown, down()),
        ("pending-up", State::PendingUp, up()),
    ] {
        let literal = VALID.replacen("applied", word, 1);
        let mut expected = record();
        expected.state = state;
        assert_eq!(Record::parse(&literal), Ok(expected.clone()));
        assert_eq!(expected.encode(), literal);
        assert_eq!(expected.pending(), Some(transition));
    }
}

#[test]
fn effort_record_percent_encoding_uses_independent_exact_spelling() {
    let mut expected = record();
    expected.windows[0].key.bucket = "weekly all".into();
    expected.windows[0].key.qualifier = Some("opus/weekly".into());
    let literal = VALID.replace("win:weekly/~/", "win:weekly%20all/opus%2Fweekly/");
    assert_eq!(Record::parse(&literal), Ok(expected.clone()));
    assert_eq!(expected.encode(), literal);
    for invalid in [
        "weekly%2fall",
        "weekly%61ll",
        "weekly%00",
        "weekly%FF",
        "weekly%",
        "weekly%0a",
    ] {
        assert!(
            Record::parse(&VALID.replace("win:weekly/~/", &format!("win:{invalid}/~/"))).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn effort_record_refuses_noncanonical_fields_incoherent_pair_and_short_windows() {
    for (old, bad) in [
        ("v1 ", "v2 "),
        ("applied", "future"),
        ("base:xhigh low:high", "base:max low:high"),
        ("base:xhigh low:high", "base:high low:medium"),
        ("slot:spawned.0", "slot:bogus"),
        ("at:80", "at:49"),
        ("at:80", "at:101"),
        ("at:80", "at:080"),
        ("ep:1700000000", "ep:0"),
        ("ep:1700000000", "ep:+1700000000"),
        ("ep:1700000000", "ep:9223372036854775808"),
        ("pid:42", "pid:0"),
        ("pid:42", "pid:4294967296"),
        ("agent:scout", "agent:_scout"),
        ("tool:codex", "tool:muse"),
        ("launch:launch-1", "launch:launch/1"),
        ("client:-", "client:_bad"),
        ("model:gpt-6.1-sol", "model:bad%pin"),
        ("profile:solx", "profile:"),
        ("/10080/", "/~/"),
        ("/10080/", "/300/"),
        ("/10080/", "/1439/"),
        ("/10080/", "/010080/"),
        ("/1700000000/", "/0/"),
        ("/1700000100", "/-1"),
        ("v1 applied", "v1  applied"),
    ] {
        assert!(
            Record::parse(&VALID.replace(old, bad)).is_err(),
            "{old} -> {bad}"
        );
    }
    assert!(Record::parse(&VALID.replace(SID, "11111111-1111-4111-8111-11111111111X")).is_err());
    assert!(Record::parse(&VALID.replace(CONV, "not-a-uuid")).is_err());
    assert!(Record::parse(&format!("{VALID} extra:value")).is_err());
    assert!(Record::parse(&format!("{VALID}\n")).is_err());
    assert!(Record::parse(&VALID.replace("v1 applied", "v1\tapplied")).is_err());
}

#[test]
fn effort_record_bounds_and_duplicate_window_identity() {
    for (old, oversized) in [
        ("agent:scout", format!("agent:{}", "a".repeat(65))),
        ("profile:solx", format!("profile:{}", "p".repeat(65))),
        ("client:-", format!("client:{}", "c".repeat(65))),
        ("launch:launch-1", format!("launch:{}", "l".repeat(65))),
        ("model:gpt-6.1-sol", format!("model:{}", "m".repeat(257))),
        ("win:weekly/", format!("win:{}/", "w".repeat(65))),
    ] {
        assert!(
            Record::parse(&VALID.replace(old, &oversized)).is_err(),
            "bound {old}"
        );
    }
    let at_bound = VALID
        .replace("client:-", &format!("client:{}", "c".repeat(64)))
        .replace("pid:42", "pid:4294967295")
        .replace("/10080/", "/4294967295/");
    assert!(
        Record::parse(&at_bound).is_ok(),
        "64-cell configured clients are not display cells"
    );
    let duplicate = format!("{VALID},weekly/~/10080/1700000001/1700000200");
    assert!(
        Record::parse(&duplicate).is_err(),
        "duplicate key cannot hide another stamp"
    );
    let nine = (0..9)
        .map(|i| format!("w{i}/~/10080/1700000000/~"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(Record::parse(&VALID.replace("weekly/~/10080/1700000000/1700000100", &nine)).is_err());
    assert!(Record::parse(&"x".repeat(4097)).is_err());
}

#[test]
fn effort_stored_absent_damage_and_foreign_slot_never_become_new_episode() {
    let row = format!("effort.spawned.0={VALID}\n");
    assert_eq!(
        stored(row.as_bytes(), "spawned.0"),
        Stored::Present(record())
    );
    assert_eq!(
        stored(b"other=value\neffort.spawned.01=unknown\n", "spawned.0"),
        Stored::Absent
    );
    assert_eq!(
        stored(b"effort.spawned.0\n", "spawned.0"),
        Stored::Damaged(Damage::Bare)
    );
    for duplicate in [
        format!("{row}{row}"),
        format!("effort.spawned.0\n{row}"),
        format!("{row}effort.spawned.0\n"),
    ] {
        assert_eq!(
            stored(duplicate.as_bytes(), "spawned.0"),
            Stored::Damaged(Damage::Duplicated)
        );
    }
    assert_eq!(
        stored(b"effort.spawned.0=v2 nonsense\n", "spawned.0"),
        Stored::Damaged(Damage::Version)
    );
    assert_eq!(
        stored(b"effort.spawned.0=\xff\n", "spawned.0"),
        Stored::Damaged(Damage::Encoding)
    );
    assert_eq!(
        stored(format!("effort.main={VALID}\n").as_bytes(), "main"),
        Stored::Damaged(Damage::Field("slot"))
    );
}

#[test]
fn effort_guard_checks_stable_identity_not_session_name_or_prior_pid() {
    let missing = RecordedClient::Missing;
    let invalid = RecordedClient::Invalid;
    let mut now = SeatNow {
        session_id: SID,
        slot: "spawned.0",
        agent: "scout",
        tool: ToolKind::Codex,
        conversation: Some(CONV),
        profile: "solx",
        client: &missing,
        model: Some("gpt-6.1-sol"),
        baseline: Some(Level::Xhigh),
    };
    let mut r = record();
    r.launch = "prior-launch".into();
    r.pid = 1234;
    assert_eq!(
        guard(&r, &now),
        Ok(()),
        "old process provenance is rebound only by later lifecycle owner"
    );
    for (case, expected) in [
        (0, Foreign::Session),
        (1, Foreign::Slot),
        (2, Foreign::Agent),
        (3, Foreign::Tool),
        (4, Foreign::Conversation),
        (5, Foreign::Profile),
        (6, Foreign::Client),
        (7, Foreign::Model),
        (8, Foreign::Baseline),
    ] {
        now.session_id = SID;
        now.slot = "spawned.0";
        now.agent = "scout";
        now.tool = ToolKind::Codex;
        now.conversation = Some(CONV);
        now.profile = "solx";
        now.client = &missing;
        now.model = Some("gpt-6.1-sol");
        now.baseline = Some(Level::Xhigh);
        match case {
            0 => now.session_id = OTHER,
            1 => now.slot = "spawned.1",
            2 => now.agent = "other",
            3 => now.tool = ToolKind::Claude,
            4 => now.conversation = None,
            5 => now.profile = "new-profile",
            6 => now.client = &invalid,
            7 => now.model = None,
            _ => now.baseline = None,
        }
        assert_eq!(guard(&r, &now), Err(expected), "guard leg {case}");
    }
}

#[test]
fn effort_fold_reservation_alone_spends_direction_and_survives_rename_restart() {
    let mut events = opened();
    events.push(event("effort-reserved", "1700000000-down", "xhigh->high"));
    let mut expected = episode();
    expected.down = Leg::Reserved(down());
    assert_eq!(folded(&events), Fold::Open(expected.clone()));
    assert_eq!(
        folded(&events),
        Fold::Open(expected),
        "restart reads durable reservation, no new attempt"
    );
    events.push(event("effort-reserved", "1700000000-down", "xhigh->high"));
    assert!(
        matches!(folded(&events), Fold::Doubt(_)),
        "second down reservation refuses"
    );
}

#[test]
fn effort_fold_exact_pairs_and_inverse_restore_are_bound_to_reservation() {
    for bad in [
        event("effort-applied", "1700000000-down", "max->xhigh"),
        event(
            "effort-failed",
            "1700000000-down",
            "max->xhigh; key refused",
        ),
    ] {
        let mut events = opened();
        events.push(event("effort-reserved", "1700000000-down", "xhigh->high"));
        events.push(bad);
        assert!(
            matches!(folded(&events), Fold::Doubt(_)),
            "outcome pair must equal reservation"
        );
    }
    let mut events = applied();
    events.push(event("effort-reserved", "1700000000-up", "xhigh->max"));
    assert!(
        matches!(folded(&events), Fold::Doubt(_)),
        "up must invert completed down pair"
    );
}

#[test]
fn effort_fold_outcomes_require_reservation_and_matching_duplicates_are_inert() {
    let mut events = opened();
    events.push(event("effort-applied", "1700000000-down", "xhigh->high"));
    assert!(matches!(folded(&events), Fold::Doubt(_)));
    let mut events = applied();
    events.push(event(
        "effort-applied",
        "1700000000-down",
        "xhigh->high; reconciled",
    ));
    let mut expected = episode();
    expected.down = Leg::Done(down());
    assert_eq!(folded(&events), Fold::Open(expected));
    events.push(event(
        "effort-failed",
        "1700000000-down",
        "xhigh->high; conflicting",
    ));
    assert!(matches!(folded(&events), Fold::Doubt(_)));
}

#[test]
fn effort_fold_closed_restore_duplicates_never_close_a_later_episode() {
    let mut events = applied();
    events.push(event("effort-reserved", "1700000000-up", "high->xhigh"));
    events.push(event("effort-restored", "1700000000-up", "high->xhigh"));
    events.push(event(
        "effort-restored",
        "1700000000-up",
        "high->xhigh; reconciled",
    ));
    assert_eq!(folded(&events), Fold::Armed);
    let mut next = event("effort-episode", "1700000001", "threshold 90");
    next.ts = ae::time::Timestamp::from_epoch(EP + 1);
    events.push(next);
    events.push(event("effort-restored", "1700000000-up", "high->xhigh"));
    let mut expected = episode();
    expected.key = EP + 1;
    expected.threshold = 90;
    assert_eq!(folded(&events), Fold::Open(expected));
    events.push(event("effort-restored", "1700000000-up", "xhigh->max"));
    assert!(matches!(folded(&events), Fold::Doubt(_)));
}

#[test]
fn effort_fold_held_reason_dedup_and_failed_up_remain_terminal() {
    let mut events = applied();
    events.push(event("effort-held", "1700000000", "spend-cap"));
    events.push(event("effort-held", "1700000000", "spend-cap"));
    let mut expected = episode();
    expected.down = Leg::Done(down());
    expected.named = vec!["spend-cap".into()];
    assert_eq!(folded(&events), Fold::Open(expected.clone()));
    events.push(event("effort-reserved", "1700000000-up", "high->xhigh"));
    events.push(event(
        "effort-failed",
        "1700000000-up",
        "high->xhigh; receipt unknown",
    ));
    expected.up = Leg::Failed(up());
    assert_eq!(folded(&events), Fold::Open(expected));
    events.push(event("effort-rearmed", "1700000000", "relief"));
    assert!(
        matches!(folded(&events), Fold::Doubt(_)),
        "failed up cannot loop on relief"
    );
}

#[test]
fn effort_fold_rearm_refuses_owned_or_pending_override_and_allows_yielded_done() {
    for mut events in [applied(), {
        let mut e = opened();
        e.push(event("effort-reserved", "1700000000-down", "xhigh->high"));
        e
    }] {
        events.push(event("effort-rearmed", "1700000000", "relief"));
        assert!(matches!(folded(&events), Fold::Doubt(_)));
    }
    let mut events = applied();
    events.push(event("effort-yielded", "1700000000", "manual-effort"));
    events.push(event("effort-rearmed", "1700000000", "relief"));
    assert_eq!(folded(&events), Fold::Armed);
}

#[test]
fn effort_fold_rejects_bad_refs_summary_direction_and_out_of_order_records() {
    for bad in [
        event("effort-reserved", "1700000000-down", "max->high"),
        event("effort-reserved", "1700000000-up", "high->xhigh"),
        event("effort-reserved", "1700000000-down", "high->xhigh"),
        event("effort-held", "1700000000", "arbitrary"),
        event("effort-held", "01700000000", "spend-cap"),
        event("effort-held", "1700000001", "spend-cap"),
        event("effort-yielded", "1700000000", "spend-cap"),
        event("effort-episode", "1700000000", "threshold 80"),
    ] {
        let mut events = opened();
        events.push(bad);
        assert!(
            matches!(folded(&events), Fold::Doubt(_)),
            "damaged effort trace"
        );
    }
    for summary in [
        "threshold 49",
        "threshold 101",
        "threshold 080",
        "threshold",
        "threshold 80 extra",
    ] {
        assert!(
            matches!(
                folded(&[event("effort-episode", "1700000000", summary)]),
                Fold::Doubt(_)
            ),
            "{summary}"
        );
    }
}

#[test]
fn effort_fold_ignores_foreign_actor_or_slot_and_lifecycle_reset_is_explicit() {
    let mut forged = event("effort-episode", "1700000000", "threshold 80");
    forged.actor = "scout".into();
    assert_eq!(folded(&[forged]), Fold::Armed);
    let foreign = Event::parse_line(&format!(
        r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"effort-episode","target":"scout","target_slot":"spawned.1","target_session":"after-rename","target_session_id":"{SID}","ref":"1700000000","summary":"threshold 80"}}"#
    )).expect("foreign fixture");
    assert_eq!(folded(&[foreign]), Fold::Armed);
    for action in ["spawn", "reseat"] {
        let mut events = applied();
        events.push(event(action, "", "new incarnation"));
        assert_eq!(
            folded(&events),
            Fold::Armed,
            "{action} clears old ownership episode"
        );
    }
}

#[test]
fn effort_actions_are_alert_undefined_and_cannot_end_quiet_declarations() {
    for action in [
        "effort-episode",
        "effort-reserved",
        "effort-applied",
        "effort-restored",
        "effort-failed",
        "effort-held",
        "effort-yielded",
        "effort-rearmed",
    ] {
        let e = event(action, "1700000000", "synthetic");
        assert_eq!(e.alert_meaning(), AlertMeaning::Undefined);
        for state in ["done", "waiting-agent", "blocked", "waiting-user"] {
            assert!(
                !ae::watchdog::ends_quiet(state, &e, false, &[]),
                "{action} must not end {state}"
            );
        }
    }
}

#[test]
fn effort_decide_armed_opens_at_threshold_and_holds_without_long_reading() {
    let state_fold = Fold::Armed;
    let absent = Stored::Absent;
    let configured_twin = Ok(twin());
    assert_eq!(
        decide(&inputs(
            &scope(79.9),
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Idle
    );
    assert_eq!(
        decide(&inputs(
            &scope(80.0),
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Open { threshold: 80 }
    );
    let quota_scope = Scope {
        windows: vec![window("short", 95.0, Some(300))],
        spend_capped: false,
    };
    assert_eq!(
        decide(&inputs(
            &quota_scope,
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Hold(Hold::Unread)
    );
}

#[test]
fn effort_decide_doubt_damage_limits_and_missing_scope_never_authorize_change() {
    let quota_scope = scope(90.0);
    let state_fold = Fold::Armed;
    let absent = Stored::Absent;
    let configured_twin = Ok(twin());
    let mut input = inputs(&quota_scope, &state_fold, &absent, &configured_twin);
    input.limited = true;
    assert_eq!(decide(&input), Decision::Hold(Hold::Limit));
    input.limited = false;
    input.scope = None;
    assert_eq!(decide(&input), Decision::Hold(Hold::Unread));
    let doubt = Fold::Doubt("damaged trace");
    assert_eq!(
        decide(&inputs(&quota_scope, &doubt, &absent, &configured_twin)),
        Decision::Hold(Hold::Doubt)
    );
    let damaged = Stored::Damaged(Damage::Duplicated);
    assert_eq!(
        decide(&inputs(
            &quota_scope,
            &state_fold,
            &damaged,
            &configured_twin
        )),
        Decision::Hold(Hold::Damaged)
    );
    let present = Stored::Present(record());
    assert_eq!(
        decide(&inputs(
            &quota_scope,
            &state_fold,
            &present,
            &configured_twin
        )),
        Decision::Hold(Hold::Doubt),
        "record without episode cannot be adopted"
    );
}

#[test]
fn effort_decide_down_requires_live_baseline_and_preserves_episode_threshold() {
    let quota_scope = scope(80.0);
    let state_fold = Fold::Open(episode());
    let absent = Stored::Absent;
    let configured_twin = Ok(twin());
    assert_eq!(
        decide(&inputs(
            &quota_scope,
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Down {
            twin: twin(),
            windows: vec![pressured("weekly", Some(EP + 100))]
        }
    );
    let mut input = inputs(&quota_scope, &state_fold, &absent, &configured_twin);
    input.live = None;
    assert_eq!(decide(&input), Decision::Hold(Hold::Unread));
    input.live = Some(Level::High);
    assert_eq!(
        decide(&input),
        Decision::Yield("manual-effort"),
        "initial manual choice owns this episode"
    );
    input.live = Some(Level::Xhigh);
    input.at = 90;
    assert_eq!(
        decide(&input),
        Decision::Down {
            twin: twin(),
            windows: vec![pressured("weekly", Some(EP + 100))]
        }
    );
    input.new_attempts = false;
    assert_eq!(decide(&input), Decision::Hold(Hold::Disabled));
}

#[test]
fn effort_decide_near_relief_static_skips_and_no_long_rearm() {
    let state_fold = Fold::Open(episode());
    let absent = Stored::Absent;
    let configured_twin = Ok(twin());
    assert_eq!(
        decide(&inputs(
            &scope(79.0),
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Hold(Hold::Near)
    );
    assert_eq!(
        decide(&inputs(
            &scope(74.9),
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Rearm
    );
    let no_twin = Err(TwinRefusal::NoTwin(Level::High));
    assert_eq!(
        decide(&inputs(&scope(80.0), &state_fold, &absent, &no_twin)),
        Decision::Name("no-twin")
    );
    let mut episode_state = episode();
    episode_state.named.push("no-twin".into());
    let named = Fold::Open(episode_state);
    assert_eq!(
        decide(&inputs(&scope(80.0), &named, &absent, &no_twin)),
        Decision::Hold(Hold::Skipped)
    );
    assert_eq!(
        decide(&inputs(&scope(80.0), &named, &absent, &configured_twin)),
        Decision::Down {
            twin: twin(),
            windows: vec![pressured("weekly", Some(EP + 100))]
        },
        "new valid configured twin invalidates only no-key skip"
    );
    let mut episode_state = episode();
    episode_state.named.push("unrecordable".into());
    let named = Fold::Open(episode_state);
    assert_eq!(
        decide(&inputs(&scope(80.0), &named, &absent, &configured_twin)),
        Decision::Hold(Hold::Skipped),
        "changed quota rows alone never invalidate static skip"
    );
    let huge = Scope {
        windows: (0..9)
            .map(|input| window(&format!("w{input}"), 90.0, Some(10_080)))
            .collect(),
        spend_capped: false,
    };
    assert_eq!(
        decide(&inputs(&huge, &state_fold, &absent, &configured_twin)),
        Decision::Name("unrecordable")
    );
    let no_long = Scope {
        windows: vec![window("short", 1.0, Some(300))],
        spend_capped: false,
    };
    assert_eq!(
        decide(&inputs(&no_long, &state_fold, &absent, &configured_twin)),
        Decision::Hold(Hold::Unread)
    );
}

#[test]
fn effort_decide_spend_cap_names_once_across_restart_and_spends_no_attempt() {
    let mut quota_scope = scope(80.0);
    quota_scope.spend_capped = true;
    let state_fold = Fold::Armed;
    let absent = Stored::Absent;
    let configured_twin = Ok(twin());
    assert_eq!(
        decide(&inputs(
            &quota_scope,
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Open { threshold: 80 }
    );
    let events = opened();
    let state_fold = folded(&events);
    assert_eq!(
        decide(&inputs(
            &quota_scope,
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Name("spend-cap")
    );
    let mut events = events;
    events.push(event("effort-held", "1700000000", "spend-cap"));
    let mut expected = episode();
    expected.named = vec!["spend-cap".into()];
    assert_eq!(
        folded(&events),
        Fold::Open(expected),
        "no Down or Up reservation spent"
    );
    for _ in 0..3 {
        assert_eq!(
            decide(&inputs(
                &quota_scope,
                &folded(&events),
                &absent,
                &configured_twin
            )),
            Decision::Hold(Hold::SpendCap)
        );
    }
    quota_scope.spend_capped = false;
    assert_eq!(
        decide(&inputs(
            &quota_scope,
            &folded(&events),
            &absent,
            &configured_twin
        )),
        Decision::Down {
            twin: twin(),
            windows: vec![pressured("weekly", Some(EP + 100))]
        },
        "cap clearance reconsiders same unspent episode"
    );
}

#[test]
fn effort_decide_reservations_reconcile_before_changed_quota_or_human_yield() {
    let quota_scope = scope(90.0);
    let absent = Stored::Absent;
    let configured_twin = Ok(twin());
    for direction in [Direction::Down, Direction::Up] {
        let mut episode_state = episode();
        if direction == Direction::Down {
            episode_state.down = Leg::Reserved(down());
        } else {
            episode_state.down = Leg::Done(down());
            episode_state.up = Leg::Reserved(up());
        }
        episode_state.yielded = true;
        let state_fold = Fold::Open(episode_state);
        let mut input = inputs(&quota_scope, &state_fold, &absent, &configured_twin);
        input.scope = None;
        assert_eq!(
            decide(&input),
            Decision::Reconcile(direction),
            "reservation cannot be re-armed from relief/silence"
        );
    }
}

#[test]
fn effort_decide_failed_down_requires_relief_then_a_new_crossing() {
    let mut episode_state = episode();
    episode_state.down = Leg::Failed(down());
    let state_fold = Fold::Open(episode_state);
    let absent = Stored::Absent;
    let configured_twin = Ok(twin());
    for percent in [80.0, 79.0, 80.0, 75.0] {
        assert_eq!(
            decide(&inputs(
                &scope(percent),
                &state_fold,
                &absent,
                &configured_twin
            )),
            Decision::Hold(Hold::Spent)
        );
    }
    assert_eq!(
        decide(&inputs(
            &scope(74.9),
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Rearm
    );
    let empty = Scope {
        windows: vec![],
        spend_capped: false,
    };
    assert_eq!(
        decide(&inputs(&empty, &state_fold, &absent, &configured_twin)),
        Decision::Hold(Hold::Unread)
    );
    let armed = Fold::Armed;
    assert_eq!(
        decide(&inputs(&scope(79.0), &armed, &absent, &configured_twin)),
        Decision::Idle
    );
    assert_eq!(
        decide(&inputs(&scope(80.0), &armed, &absent, &configured_twin)),
        Decision::Open { threshold: 80 }
    );
}

#[test]
fn effort_decide_yielded_record_retires_before_rearm_and_never_adopts_manual_effort() {
    let mut episode_state = episode();
    episode_state.down = Leg::Done(down());
    episode_state.yielded = true;
    let state_fold = Fold::Open(episode_state);
    let present = Stored::Present(record());
    let absent = Stored::Absent;
    let configured_twin = Ok(twin());
    assert_eq!(
        decide(&inputs(
            &scope(80.0),
            &state_fold,
            &present,
            &configured_twin
        )),
        Decision::Retire
    );
    assert_eq!(
        decide(&inputs(
            &scope(80.0),
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Hold(Hold::Yielded)
    );
    assert_eq!(
        decide(&inputs(
            &scope(74.9),
            &state_fold,
            &absent,
            &configured_twin
        )),
        Decision::Rearm
    );
    let no_long = Scope {
        windows: vec![],
        spend_capped: false,
    };
    assert_eq!(
        decide(&inputs(&no_long, &state_fold, &absent, &configured_twin)),
        Decision::Hold(Hold::Unread)
    );
}

#[test]
fn effort_decide_applied_recovers_once_and_never_takes_a_second_downstep() {
    let mut episode_state = episode();
    episode_state.down = Leg::Done(down());
    let state_fold = Fold::Open(episode_state);
    let present = Stored::Present(record());
    let configured_twin = Ok(twin());
    let high = scope(90.0);
    let mut input = inputs(&high, &state_fold, &present, &configured_twin);
    input.live = Some(Level::High);
    assert_eq!(decide(&input), Decision::Hold(Hold::Pressure));
    let low = scope(74.9);
    input.scope = Some(&low);
    assert_eq!(decide(&input), Decision::Up);
    let near = scope(75.0);
    input.scope = Some(&near);
    assert_eq!(decide(&input), Decision::Hold(Hold::Pressure));
    input.scope = Some(&low);
    input.new_attempts = false;
    input.at = 90;
    assert_eq!(
        decide(&input),
        Decision::Up,
        "new-attempt threshold edits cannot kill recorded recovery"
    );
}

#[test]
fn effort_decide_applied_missing_guard_live_or_identity_change_holds_or_yields() {
    let mut episode_state = episode();
    episode_state.down = Leg::Done(down());
    let state_fold = Fold::Open(episode_state);
    let present = Stored::Present(record());
    let configured_twin = Ok(twin());
    let low = scope(74.9);
    let mut input = inputs(&low, &state_fold, &present, &configured_twin);
    input.live = Some(Level::High);
    input.guard = None;
    assert_eq!(decide(&input), Decision::Hold(Hold::Unread));
    input.guard = Some(Err(Foreign::Model));
    assert_eq!(decide(&input), Decision::Yield("baseline-changed"));
    input.guard = Some(Ok(()));
    input.live = None;
    assert_eq!(decide(&input), Decision::Hold(Hold::Unread));
    input.live = Some(Level::Xhigh);
    assert_eq!(decide(&input), Decision::Yield("manual-effort"));
}

#[test]
fn effort_decide_applied_meta_and_journal_must_agree_on_epoch_threshold_pair_state() {
    let mut episode_state = episode();
    episode_state.down = Leg::Done(down());
    let state_fold = Fold::Open(episode_state);
    let low = scope(74.9);
    let configured_twin = Ok(twin());
    for case in 0..4 {
        let mut meta_record = record();
        match case {
            0 => meta_record.episode += 1,
            1 => meta_record.threshold = 90,
            2 => {
                meta_record.base = Level::Max;
                meta_record.low = Level::Xhigh;
            }
            _ => meta_record.state = State::PendingUp,
        }
        let present = Stored::Present(meta_record);
        let mut input = inputs(&low, &state_fold, &present, &configured_twin);
        input.live = Some(Level::High);
        assert_eq!(
            decide(&input),
            Decision::Hold(Hold::Doubt),
            "meta disagreement {case}"
        );
    }
    let absent = Stored::Absent;
    assert_eq!(
        decide(&inputs(&low, &state_fold, &absent, &configured_twin)),
        Decision::Hold(Hold::Doubt)
    );
}

#[test]
fn effort_decide_failed_up_is_terminal_even_through_repeated_relief() {
    let mut episode_state = episode();
    episode_state.down = Leg::Done(down());
    episode_state.up = Leg::Failed(up());
    let state_fold = Fold::Open(episode_state);
    let low = scope(74.9);
    let present = Stored::Present(record());
    let configured_twin = Ok(twin());
    let mut input = inputs(&low, &state_fold, &present, &configured_twin);
    input.live = Some(Level::High);
    for _ in 0..3 {
        assert_eq!(decide(&input), Decision::Hold(Hold::TerminalUp));
    }
}

#[test]
fn effort_decide_spend_cap_holds_applied_reset_only_recovery_without_spending_up() {
    let mut episode_state = episode();
    episode_state.down = Leg::Done(down());
    let state_fold = Fold::Open(episode_state);
    let mut meta_record = record();
    meta_record.windows[0].resets_at = Some(NOW);
    let present = Stored::Present(meta_record);
    let configured_twin = Ok(twin());
    let mut quota_scope = Scope {
        windows: vec![window("short", 0.0, Some(300))],
        spend_capped: true,
    };
    let mut input = inputs(&quota_scope, &state_fold, &present, &configured_twin);
    input.live = Some(Level::High);
    assert_eq!(decide(&input), Decision::Name("spend-cap"));
    let mut episode_state = episode();
    episode_state.down = Leg::Done(down());
    episode_state.named.push("spend-cap".into());
    let named = Fold::Open(episode_state);
    input.fold = &named;
    assert_eq!(decide(&input), Decision::Hold(Hold::SpendCap));
    quota_scope.spend_capped = false;
    let mut input = inputs(&quota_scope, &named, &present, &configured_twin);
    input.live = Some(Level::High);
    assert_eq!(
        decide(&input),
        Decision::Up,
        "B3 reset-only survives no current long row after cap clearance"
    );
}
