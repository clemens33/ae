//! #318 persisted-record acceptance from the ratified S2 external grammar.
//! Oracle: lead ae-20261113T081825Z-f97c3a78. No runtime policy is simulated.
#![allow(clippy::expect_used, reason = "literal independent record fixtures")]

use ae::effort::{Damage, Level, Record, Stored, stored};

const TAIL: &str = "at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:old-token pid:42 conv:22222222-2222-4222-8222-222222222222 profile:custom client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700000100";

fn v1(base: &str, low: &str) -> String {
    format!("v1 applied slot:main base:{base} low:{low} {TAIL}")
}

fn v2(base: &str, low: &str, step: &str) -> String {
    format!("v2 applied slot:main base:{base} low:{low} step:{step} {TAIL}")
}

#[test]
fn p2_v1_retains_only_legacy_pairs_and_its_exact_canonical_bytes() {
    for (base, low) in [("max", "xhigh"), ("xhigh", "high")] {
        let text = v1(base, low);
        let record = Record::parse(&text).expect("legacy record stays readable");
        assert_eq!(record.encode(), text, "legacy bytes remain old-reader safe");
        assert_eq!(Record::parse(&record.encode()), Ok(record));
    }
    for (base, low) in [("high", "medium"), ("ultra", "max"), ("xhigh", "medium")] {
        assert_eq!(
            Record::parse(&v1(base, low)),
            Err(Damage::Incoherent),
            "v1 cannot claim a new-tier or two-step move"
        );
    }
}

#[test]
fn p2_v2_accepts_only_nonlegacy_adjacent_one_or_two_step_pairs() {
    for (base, low, step) in [
        ("ultra", "max", "1"),
        ("high", "medium", "1"),
        ("medium", "low", "1"),
        ("low", "minimal", "1"),
        ("ultra", "xhigh", "2"),
        ("max", "high", "2"),
        ("xhigh", "medium", "2"),
        ("high", "low", "2"),
        ("medium", "minimal", "2"),
    ] {
        let text = v2(base, low, step);
        let record = Record::parse(&text).expect("literal adjacent-tier v2 record");
        assert_eq!(record.base.word(), base);
        assert_eq!(record.low.word(), low);
        assert_eq!(
            record.encode(),
            text,
            "v2 field order and step are canonical"
        );
        assert_eq!(Record::parse(&record.encode()), Ok(record.clone()));
        let meta = format!("effort.main={text}\n");
        assert_eq!(stored(meta.as_bytes(), "main"), Stored::Present(record));
    }
}

#[test]
fn p2_v2_legacy_one_step_pairs_are_noncanonical_instead_of_silently_reencoded() {
    for (base, low) in [("max", "xhigh"), ("xhigh", "high")] {
        assert_eq!(Record::parse(&v2(base, low, "1")), Err(Damage::Shape));
    }
}

#[test]
fn p2_v2_refuses_bad_step_counts_nonadjacent_pairs_and_ambiguous_fields() {
    for step in ["0", "3", "01", "-1", "two", ""] {
        assert!(
            Record::parse(&v2("xhigh", "medium", step)).is_err(),
            "hostile step {step:?}"
        );
    }
    for (base, low, step) in [
        ("xhigh", "high", "2"),
        ("xhigh", "low", "2"),
        ("high", "high", "1"),
        ("medium", "high", "1"),
        ("minimal", "low", "1"),
    ] {
        assert_eq!(Record::parse(&v2(base, low, step)), Err(Damage::Incoherent));
    }
    let good = v2("xhigh", "medium", "2");
    for text in [
        good.replace(" step:2", ""),
        good.replace("step:2", "step:2 step:2"),
        format!("{good} step:2"),
        good.replace("low:medium step:2", "step:2 low:medium"),
        good.replace("v2 ", "v3 "),
    ] {
        assert!(Record::parse(&text).is_err(), "no field ambiguity: {text}");
    }
    assert_eq!(Record::parse(&good).expect("true v2").low, Level::Medium);
}

fn two_step_events(applied_second: bool) -> Vec<ae::events::Event> {
    [
        ("effort-episode", "1700000000", "threshold 80"),
        ("effort-reserved", "1700000000-down", "xhigh->high"),
        ("effort-applied", "1700000000-down", "xhigh->high"),
        ("effort-reserved", "1700000000-down2", "high->medium"),
        ("effort-applied", "1700000000-down2", "high->medium"),
    ].into_iter().filter(|(action, reference, _)| applied_second || *action != "effort-applied" || !reference.ends_with("-down2")).map(|(action, reference, summary)| {
        ae::events::Event::parse_line(&format!(r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"{action}","target":"lead","target_slot":"main","target_session":"fixture","target_session_id":"11111111-1111-4111-8111-111111111111","ref":"{reference}","summary":"{summary}"}}"#)).expect("literal two-step history")
    }).collect()
}

#[test]
fn p2_v2_pending_moves_are_from_current_tier_down_and_directly_to_baseline_up() {
    use ae::effort::{Direction, Transition};
    for (state, direction, from, to) in [
        ("pending-down", Direction::Down, Level::High, Level::Medium),
        ("restart-down", Direction::Down, Level::High, Level::Medium),
        ("pending-up", Direction::Up, Level::Medium, Level::Xhigh),
        ("restart-up", Direction::Up, Level::Medium, Level::Xhigh),
    ] {
        let text = v2("xhigh", "medium", "2").replacen(" applied ", &format!(" {state} "), 1);
        let record = Record::parse(&text).expect("literal two-step pending or restart state");
        assert_eq!(
            record.pending(),
            Some(Transition {
                direction,
                from,
                to
            })
        );
    }
    assert_eq!(
        Record::parse(&v2("xhigh", "medium", "2"))
            .expect("applied v2")
            .pending(),
        None
    );
}

#[test]
fn p2_retained_resume_replays_the_v2_current_tier_for_applied_and_reserved_step_two() {
    use ae::effort::{Fold, Resume, Switch, fold, on_resume};
    for (state, second_applied) in [("applied", true), ("restart-down", false)] {
        let text = v2("xhigh", "medium", "2").replacen(" applied ", &format!(" {state} "), 1);
        let record = Record::parse(&text).expect("literal retained two-step record");
        let events = two_step_events(second_applied);
        let ledger = fold(
            &events,
            ae::events::SessionKey::new("fixture", "11111111-1111-4111-8111-111111111111"),
            "main",
            "lead",
        );
        assert!(
            matches!(ledger, Fold::Open(_)),
            "literal two-step trace remains an open episode: {ledger:?}"
        );
        assert_eq!(
            on_resume(
                &Stored::Present(record),
                Ok(()),
                Some(&ledger),
                Switch::On,
                true
            ),
            Resume::Replay(Level::Medium),
            "same-conversation {state} must replay the second tier, never high or xhigh"
        );
    }
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "owned literal-v2 retirement fixture"
)]
fn p2_retiring_a_v2_claim_keeps_the_episode_reference_in_one_durable_ending() {
    use std::fmt::Write as _;
    let scratch = super::cli::OwnedScratch::root("efv", "retire");
    let dir = scratch.join("fixture");
    std::fs::create_dir_all(&dir).expect("owned session");
    let value = v2("xhigh", "medium", "2")
        .replace("slot:main", "slot:spawned.0")
        .replace("agent:lead", "agent:scout");
    let meta = format!(
        "session=fixture\nsession_id=11111111-1111-4111-8111-111111111111\nseat.spawned.0=scout\nprofile.spawned.0=custom\nagent_bin.spawned.0=codex\nlaunch_id.spawned.0=old-token\neffort.spawned.0={value}\n"
    );
    assert_eq!(
        stored(meta.as_bytes(), "spawned.0").episode(),
        Some(1_700_000_000),
        "Stored::episode reads the v2 identity"
    );
    std::fs::write(dir.join("meta"), meta).expect("literal v2 seat");
    let mut journal = String::new();
    for (action, reference, summary) in [
        ("effort-episode", "1700000000", "threshold 80"),
        ("effort-reserved", "1700000000-down", "xhigh->high"),
        ("effort-applied", "1700000000-down", "xhigh->high"),
        ("effort-reserved", "1700000000-down2", "high->medium"),
        ("effort-applied", "1700000000-down2", "high->medium"),
    ] {
        writeln!(journal, r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"{action}","target":"scout","target_slot":"spawned.0","target_session":"fixture","target_session_id":"11111111-1111-4111-8111-111111111111","ref":"{reference}","summary":"{summary}"}}"#).expect("literal two-step journal");
    }
    std::fs::write(dir.join("events.jsonl"), journal).expect("prior v2 history");
    ae::identity::remove_seat_slot(&dir, "scout").expect("retire two-step seat");
    let events: Vec<_> = std::fs::read_to_string(dir.join("events.jsonl"))
        .expect("durable journal")
        .lines()
        .map(|line| ae::events::Event::parse_line(line).expect("complete event"))
        .collect();
    let endings: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                event.action.as_str(),
                "effort-discarded" | "effort-restored"
            )
        })
        .collect();
    assert_eq!(endings.len(), 1);
    assert_eq!(endings[0].action, "effort-discarded");
    assert_eq!(endings[0].reference.as_deref(), Some("1700000000"));
    assert!(
        endings[0]
            .summary
            .as_deref()
            .expect("reason")
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word == "retire")
    );
    let remaining = std::fs::read_to_string(dir.join("meta")).expect("published removal");
    assert!(
        !remaining
            .lines()
            .any(|line| line.starts_with("effort.spawned.0=") || line == "seat.spawned.0=scout")
    );
    assert!(
        matches!(
            ae::effort::fold(
                &events,
                ae::events::SessionKey::new("fixture", "11111111-1111-4111-8111-111111111111"),
                "spawned.0",
                "scout"
            ),
            ae::effort::Fold::Armed
        ),
        "the consumer accepts the ending of the v2 episode"
    );
}
