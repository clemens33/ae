//! The orchestrator model guard's pure parts: the policy value, the origin proof
//! and the chooser's stamp. They hold the review findings that no scenario test
//! pins alone: a profile pin never proves the running model, a family label never
//! qualifies a candidate, and an unusable value never frees the orchestrator.

use super::*;

const OPUS_A: &str = "claude-opus-4-20250514";
const OPUS_B: &str = "claude-opus-4-20240229";
const NOW: i64 = 10_000;

fn facts<'a>(tool: ToolKind, launched: Option<&'a str>, drawn: Option<&'a str>) -> OriginFacts<'a> {
    OriginFacts {
        tool,
        launched,
        drawn,
    }
}

fn runs(profile: &str, pin: Option<&str>, tool: Option<ToolKind>, how: Provenance) -> Candidate {
    Candidate {
        profile: profile.to_owned(),
        provenance: how,
        configured: true,
        peer_latched: false,
        windows: Vec::new(),
        left_at: None,
        left_on_headroom: false,
        model: Model {
            pin: pin.map(str::to_owned),
            tool,
        },
        floor: None,
    }
}

fn stanced(stance: Option<Stance>) -> Settings {
    let mut settings = Settings::off(Vec::new());
    settings.orchestrator.stance = stance;
    settings
}

fn keep(id: &str, tool: ToolKind) -> Stance {
    Stance::Keep(Origin::Proven {
        id: id.to_owned(),
        tool,
    })
}

fn picked(settings: &Settings, route: Route<'_>, among: &[Candidate]) -> Option<String> {
    choose_seat(settings, route, among, NOW, None, None)
        .pick
        .map(|(name, _)| name)
}

#[test]
fn an_absent_policy_is_same_model_and_every_value_is_read_once() {
    let said = |raw: &str| parse_orchestrator(Ok(Some(raw.to_owned())), Ok(1));
    assert_eq!(
        parse_orchestrator(Ok(None), Ok(0)),
        (OrchestratorMove::SameModel, None)
    );
    assert_eq!(said("off"), (OrchestratorMove::Off, None));
    assert_eq!(said(" same-model "), (OrchestratorMove::SameModel, None));
    assert_eq!(said("on"), (OrchestratorMove::On, None));
}

#[test]
fn an_unusable_policy_keeps_the_orchestrator_on_its_model_with_one_note() {
    for (raw, times) in [
        (Ok(Some("yes".to_owned())), Ok(1)),
        (Ok(Some(String::new())), Ok(1)),
        (Ok(Some("off".to_owned())), Ok(2)),
        (Err("the config is unreadable".to_owned()), Ok(1)),
        (
            Ok(Some("on".to_owned())),
            Err("the key count is unreadable".to_owned()),
        ),
    ] {
        let (mode, note) = parse_orchestrator(raw.clone(), times.clone());
        assert_eq!(mode, OrchestratorMove::SameModel, "{raw:?} {times:?}");
        let note = note.expect("an unusable value says so");
        assert!(note.contains("same-model"), "{note}");
    }
}

#[test]
fn a_launch_record_proves_a_model_the_pane_does_not_contradict() {
    for drawn in [None, Some("Opus 4"), Some(OPUS_A)] {
        assert_eq!(
            orchestrator_origin(&facts(ToolKind::Claude, Some(OPUS_A), drawn)),
            Origin::Proven {
                id: OPUS_A.to_owned(),
                tool: ToolKind::Claude
            },
            "{drawn:?}"
        );
    }
}

#[test]
fn a_seat_with_no_launch_record_proves_nothing_whatever_its_pane_draws() {
    for drawn in [None, Some("Opus 4"), Some(OPUS_A)] {
        assert!(
            matches!(
                orchestrator_origin(&facts(ToolKind::Claude, None, drawn)),
                Origin::Unproven(_)
            ),
            "{drawn:?}: a profile's pin is read from a file that may have changed"
        );
    }
}

#[test]
fn a_pane_label_that_contradicts_the_record_or_a_pin_with_no_version_is_unproven() {
    assert!(matches!(
        orchestrator_origin(&facts(ToolKind::Claude, Some(OPUS_A), Some("Sonnet 4"))),
        Origin::Unproven(why) if why.contains("Sonnet 4")
    ));
    assert!(matches!(
        orchestrator_origin(&facts(ToolKind::Claude, Some("opus"), None)),
        Origin::Unproven(why) if why.contains("no version")
    ));
}

#[test]
fn a_tool_whose_footer_draws_exact_ids_is_proven_by_the_id_it_draws() {
    assert_eq!(
        orchestrator_origin(&facts(ToolKind::Codex, Some("older"), Some("gpt-5.4"))),
        Origin::Proven {
            id: "gpt-5.4".to_owned(),
            tool: ToolKind::Codex
        }
    );
    assert!(matches!(
        orchestrator_origin(&facts(ToolKind::Codex, None, None)),
        Origin::Unproven(_)
    ));
}

#[test]
fn only_a_candidate_pinning_the_same_exact_model_passes_under_same_model() {
    let among = [
        runs(
            "other-model",
            Some(OPUS_B),
            Some(ToolKind::Claude),
            Provenance::Declared,
        ),
        runs(
            "unpinned",
            None,
            Some(ToolKind::Claude),
            Provenance::Declared,
        ),
        runs(
            "other-tool",
            Some(OPUS_A),
            Some(ToolKind::Codex),
            Provenance::Declared,
        ),
        runs(
            "same",
            Some(OPUS_A),
            Some(ToolKind::Claude),
            Provenance::Declared,
        ),
    ];
    let route = Route::Move { seat_profile: "a" };
    assert_eq!(
        picked(
            &stanced(Some(keep(OPUS_A, ToolKind::Claude))),
            route,
            &among
        ),
        Some("same".to_owned())
    );
    let refused = choose_seat(
        &stanced(Some(keep(OPUS_A, ToolKind::Claude))),
        route,
        &among[..3],
        NOW,
        None,
        None,
    );
    assert_eq!(refused.pick, None);
    assert_eq!(refused.skipped.len(), 3);
    assert!(
        refused
            .skipped
            .iter()
            .all(|(_, skip)| *skip == Skip::OrchestratorModel)
    );
    assert!(
        refused
            .skipped
            .iter()
            .any(|(name, _)| name.starts_with("other-model")),
        "{:?}",
        refused.skipped
    );
}

#[test]
fn an_unproven_origin_or_the_policy_off_refuses_every_candidate_and_on_stamps_nothing() {
    let among = [runs(
        "same",
        Some(OPUS_A),
        Some(ToolKind::Claude),
        Provenance::Declared,
    )];
    let route = Route::Move { seat_profile: "a" };
    let unproven = Some(Stance::Keep(Origin::Unproven("no record".to_owned())));
    assert_eq!(picked(&stanced(unproven), route, &among), None);
    assert_eq!(picked(&stanced(Some(Stance::Off)), route, &among), None);
    assert_eq!(
        picked(&stanced(None), route, &among),
        Some("same".to_owned())
    );
}

#[test]
fn a_refusal_names_the_model_and_the_declared_row_it_comes_from() {
    let among = [runs(
        "other-model",
        Some(OPUS_B),
        Some(ToolKind::Claude),
        Provenance::Declared,
    )];
    let route = Route::Move { seat_profile: "a" };
    let choice = choose_seat(
        &stanced(Some(keep(OPUS_A, ToolKind::Claude))),
        route,
        &among,
        NOW,
        None,
        None,
    );
    let words = leg::passed_over_words(&choice);
    assert!(words.contains("other-model"), "{words}");
    assert!(words.contains("declared in [auto_reseat] a"), "{words}");
    assert!(words.contains(OPUS_B) && words.contains(OPUS_A), "{words}");
    assert!(leg::guard_refused(&choice));
    let plain = choose_seat(&stanced(None), route, &among, NOW, None, None);
    assert!(!leg::guard_refused(&plain));
}

#[test]
fn a_return_to_the_origin_follows_the_same_rule() {
    let among = [runs(
        "origin",
        Some(OPUS_B),
        Some(ToolKind::Claude),
        Provenance::Declared,
    )];
    assert_eq!(
        picked(
            &stanced(Some(keep(OPUS_A, ToolKind::Claude))),
            Route::Return { origin: "origin" },
            &among
        ),
        None,
        "a return is judged like any move"
    );
    assert_eq!(
        picked(
            &stanced(Some(keep(OPUS_B, ToolKind::Claude))),
            Route::Return { origin: "origin" },
            &among
        ),
        Some("origin".to_owned())
    );
}

mod holds {
    //! The orchestrator's own rules give a held attempt back, a bounded number of
    //! times per key, so a policy that refuses never strands a still-limited seat.

    use super::*;
    use crate::events::Event;
    use crate::time::Timestamp;

    const SESSION: &str = "guard-session";
    const SLOT: &str = "main";
    const AGENT: &str = "orchestrator";

    fn at(second: i64) -> Timestamp {
        Timestamp::from_epoch(1_700_000_000 + second)
    }

    fn record(second: i64, action: &str, summary: Option<&str>) -> Event {
        let reference = format!(r#", "ref":"{}""#, at(0));
        let summary = summary
            .map(|text| format!(r#", "summary":"{text}""#))
            .unwrap_or_default();
        Event::parse_line(&format!(
            r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"{AGENT}"{reference}{summary}}}"#,
            at(second)
        ))
        .expect("valid synthetic journal record")
    }

    fn spent(summaries: &[Option<&str>]) -> Episode {
        let mut journal =
            vec![Event::parse_line(&format!(
            r#"{{"ts":"{}","actor":"watchdog","action":"{LIMIT_ACTION}","target":"{AGENT}"}}"#,
            at(0)
        ))
        .expect("the limit record")];
        for (turn, summary) in summaries.iter().enumerate() {
            let turn = i64::try_from(turn).expect("a few turns");
            journal.push(record(10 + turn * 20, ATTEMPT_ACTION, None));
            journal.push(record(15 + turn * 20, HELD_ACTION, *summary));
        }
        episode(&journal, SESSION, SLOT, AGENT).expect("the limit episode is open")
    }

    #[test]
    fn an_ordinary_hold_spends_its_attempt() {
        let state = spent(&[Some("busy: held: a request is in flight")]);
        assert_eq!(state.attempts, 1);
        assert!(state.held);
    }

    #[test]
    fn the_orchestrators_own_rules_give_every_attempt_back_and_never_strand_the_key() {
        let changed = CauseClass::OrchestratorChanged.lead("model changed");
        let stays = CauseClass::OrchestratorStays.lead("no usable candidate");
        for (summary, word) in [(changed, "changed"), (stays, "stays")] {
            let many = vec![Some(summary.as_str()); 6];
            let state = spent(&many);
            assert_eq!(state.attempts, 0, "{word}: no number of holds spends one");
            assert!(state.held, "{word}");
            assert_eq!(state.terminal, None, "{word}");
        }
    }

    #[test]
    fn a_policy_hold_delays_the_admission_only() {
        let clear = Pane {
            frame: Frame::Clear,
            human_prompt: false,
            client_input: None,
        };
        let stays = CauseClass::OrchestratorStays.lead("no usable candidate");
        let state = spent(&[Some(stays.as_str()); 5]);
        let held_at = state.policy_hold_at.expect("the hold is dated").epoch();
        let until = held_at + POLICY_HOLD_WAIT_SECS;
        assert_eq!(state.cooling_until(until - 1), Some(until));
        assert_eq!(state.cooling_until(until), None, "the key tries again");
        // The due time and the verdict never wait on it: the decision is still an
        // attempt while the key cools, so the daemon can tell its notice.
        assert_eq!(
            decide(Some(&state), 0, &clear, until - 1),
            Decision::Attempt,
            "the cooldown is no part of the due time"
        );
        assert_eq!(
            decide(Some(&state), 0, &clear, until),
            Decision::Attempt,
            "and no number of holds uses the attempts up"
        );
        let busy = spent(&[Some("busy: held: a request is in flight"); 2]);
        assert_eq!(busy.cooling_until(busy.key.epoch()), None);
        assert_eq!(
            decide(Some(&busy), 0, &clear, busy.key.epoch() + 100_000),
            Decision::Rest,
            "an ordinary hold keeps spending its attempt"
        );
    }
}

mod pre_admission {
    //! The trigger's hold opens no attempt, so it has none to give back.

    use super::*;
    use crate::events::Event;
    use crate::time::Timestamp;

    #[test]
    fn a_policy_hold_with_no_open_attempt_refunds_nothing() {
        let stamp = |second: i64| Timestamp::from_epoch(1_700_000_000 + second);
        let line = |second: i64, action: &str, summary: Option<&str>| {
            let reference = format!(r#", "ref":"{}""#, stamp(0));
            let summary = summary
                .map(|text| format!(r#", "summary":"{text}""#))
                .unwrap_or_default();
            Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"orchestrator"{reference}{summary}}}"#,
                stamp(second)
            ))
            .expect("valid synthetic journal record")
        };
        let stays = CauseClass::OrchestratorStays.lead("no usable candidate");
        let journal = [
            Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"limit","target":"orchestrator"}}"#,
                stamp(0)
            ))
            .expect("the limit record"),
            line(10, ATTEMPT_ACTION, None),
            line(15, HELD_ACTION, Some("busy: held: a request is in flight")),
            line(30, HELD_ACTION, Some(stays.as_str())),
        ];
        let state = episode(&journal, "s", "main", "orchestrator").expect("an open episode");
        assert_eq!(
            state.attempts, 1,
            "the busy hold spent its attempt; the later pre-admission hold gave none back"
        );
    }
}

mod admission_check {
    //! The one admission check of both trigger routes and the watchdog.

    use super::*;
    use crate::events::Event;
    use crate::time::Timestamp;

    #[test]
    fn a_cooling_key_waits_for_its_admission_and_nothing_else_moves() {
        let at = |second: i64| Timestamp::from_epoch(1_700_000_000 + second);
        let line = |second: i64, action: &str, summary: Option<&str>| {
            let reference = format!(r#", "ref":"{}""#, at(0));
            let summary = summary
                .map(|text| format!(r#", "summary":"{text}""#))
                .unwrap_or_default();
            Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"orchestrator"{reference}{summary}}}"#,
                at(second)
            ))
            .expect("valid synthetic journal record")
        };
        let stays = CauseClass::OrchestratorStays.lead("no usable candidate");
        let journal = [
            Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"limit","target":"orchestrator"}}"#,
                at(0)
            ))
            .expect("the limit record"),
            line(10, ATTEMPT_ACTION, None),
            line(15, HELD_ACTION, Some(stays.as_str())),
        ];
        let state = episode(&journal, "s", "main", "orchestrator").expect("an open episode");
        let until = at(15).epoch() + POLICY_HOLD_WAIT_SECS;
        let cooling = at(16).epoch();
        assert_eq!(
            admission(Some(&state), Decision::Attempt, cooling),
            Decision::Wait { due_at: until }
        );
        assert_eq!(
            admission(Some(&state), Decision::Attempt, until),
            Decision::Attempt
        );
        for other in [
            Decision::Rest,
            Decision::InFlight,
            Decision::Overdue,
            Decision::Hold(HoldReason::Busy),
            Decision::Wait { due_at: 7 },
        ] {
            assert_eq!(admission(Some(&state), other, cooling), other, "{other:?}");
        }
        assert_eq!(
            admission(None, Decision::Attempt, cooling),
            Decision::Attempt
        );
    }
}

mod persistent {
    //! A hold the leg finds alone and keeps finding is news once it keeps coming.

    use super::*;
    use crate::events::Event;
    use crate::time::Timestamp;

    #[test]
    fn holds_become_news_at_the_third_and_the_count_outlives_the_cooldown() {
        let at = |second: i64| Timestamp::from_epoch(1_700_000_000 + second);
        let line = |second: i64, action: &str, summary: Option<&str>| {
            let reference = format!(r#", "ref":"{}""#, at(0));
            let summary = summary
                .map(|text| format!(r#", "summary":"{text}""#))
                .unwrap_or_default();
            Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"orchestrator"{reference}{summary}}}"#,
                at(second)
            ))
            .expect("valid synthetic journal record")
        };
        let held = CauseClass::OrchestratorStays.lead("Error: the carry source is missing");
        let mut journal = vec![
            Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"limit","target":"orchestrator"}}"#,
                at(0)
            ))
            .expect("the limit record"),
        ];
        let mut told = Vec::new();
        for turn in 0..3_i64 {
            journal.push(line(10 + turn * 400, ATTEMPT_ACTION, None));
            journal.push(line(15 + turn * 400, HELD_ACTION, Some(held.as_str())));
            let state = episode(&journal, "s", "main", "orchestrator").expect("open");
            told.push(state.persistent_hold().is_some());
        }
        assert_eq!(told, [false, false, true], "news at the third hold");
        let state = episode(&journal, "s", "main", "orchestrator").expect("open");
        let last = at(15 + 2 * 400).epoch();
        assert_eq!(
            state.persistent_hold(),
            Some((3, "Error: the carry source is missing")),
            "the count and the diagnostic, without the class word"
        );
        // The count is durable: it never ends with the cooldown, which gates the
        // admission alone.
        assert!(state.cooling_until(last + 1).is_some());
        assert_eq!(state.cooling_until(last + POLICY_HOLD_WAIT_SECS), None);
    }
}

/// The late gate's reach: only the orchestrator's MAIN seat is read and judged.
mod orchestrator_main {
    use super::*;

    #[test]
    fn only_the_main_seat_of_an_orchestrator_is_the_one_the_late_gate_reads() {
        let role = b"session_id=s\nmeta_agent=true\n";
        let plain = b"session_id=s\n";
        let damaged = b"session_id=s\nmeta_agent=maybe\n";
        assert!(
            leg::is_orchestrator_main(role, "main"),
            "the orchestrator's main"
        );
        assert!(
            !leg::is_orchestrator_main(role, "w1"),
            "a worker slot of the orchestrator's session is no orchestrator"
        );
        assert!(!leg::is_orchestrator_main(plain, "main"), "no role claim");
        assert!(
            !leg::is_orchestrator_main(damaged, "main"),
            "a damaged claim"
        );
    }
}

/// Who tells a hold the leg closes: the orchestrator's is the daemon's one
/// keyed notice, and a choice hold told before is never told again.
mod leg_tells {
    use super::*;

    #[test]
    fn the_leg_tells_a_hold_only_for_a_seat_that_is_no_orchestrator_and_was_not_told() {
        assert!(leg::tells_hold(false, false), "an ordinary seat's new hold");
        assert!(!leg::tells_hold(true, false), "a hold already told");
        assert!(
            !leg::tells_hold(false, true),
            "the orchestrator's hold is the daemon's to tell"
        );
        assert!(!leg::tells_hold(true, true), "both");
    }
}
