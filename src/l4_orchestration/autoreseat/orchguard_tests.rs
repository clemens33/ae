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

fn keep(id: &str, tool: ToolKind) -> Option<Stance> {
    Some(Stance::Keep(Origin::Proven {
        id: id.to_owned(),
        tool,
    }))
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
        picked(&stanced(keep(OPUS_A, ToolKind::Claude)), route, &among),
        Some("same".to_owned())
    );
    let refused = choose_seat(
        &stanced(keep(OPUS_A, ToolKind::Claude)),
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
        &stanced(keep(OPUS_A, ToolKind::Claude)),
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
            &stanced(keep(OPUS_A, ToolKind::Claude)),
            Route::Return { origin: "origin" },
            &among
        ),
        None,
        "a return is judged like any move"
    );
    assert_eq!(
        picked(
            &stanced(keep(OPUS_B, ToolKind::Claude)),
            Route::Return { origin: "origin" },
            &among
        ),
        Some("origin".to_owned())
    );
}
