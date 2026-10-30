//! #269 policy: classify persisted choice with the resume precedence, read only.

use super::{ObservedHold, Seat, observed_hold};
use crate::effort::Level;
use crate::meta::{ObservedPair, Rows};
use crate::tool::ToolKind;

const COMMAND: &str = "codex -m gpt-original -c model_reasoning_effort=xhigh";

fn seat(command: &str, rows: &str) -> Seat {
    Seat {
        session: "s".to_owned(),
        work_dir: "/w".to_owned(),
        dir_source: crate::meta::SeatProvenance::Inherited,
        config_files: Vec::new(),
        command: crate::config::IdentityConfig::resolved_snapshot(command),
        tool: ToolKind::Codex,
        harness_session: "conversation".to_owned(),
        launch_id: "launch".to_owned(),
        config_home: crate::meta::RecordedConfigHome::Missing,
        config_home_base: crate::meta::RecordedConfigHomeBase::Missing,
        agent: "lead".to_owned(),
        observed: crate::meta::observed_pair(rows.as_bytes(), "main"),
        identity_global: None,
        identity_local: None,
        recorded_tool: ToolKind::Codex,
    }
}

fn pair(model: &str, pin: Option<&str>, effort: Option<(Level, Level)>) -> Rows<ObservedPair> {
    Rows::Valid(ObservedPair {
        model: model.to_owned(),
        model_pin: pin.map(str::to_owned),
        effort,
    })
}

#[test]
fn current_model_effort_and_combined_choices_hold_as_one_pair() {
    for (model, effort, shown) in [
        ("gpt-human", None, "gpt-human"),
        (
            "gpt-original",
            Some((Level::High, Level::Xhigh)),
            "gpt-original (high)",
        ),
        (
            "gpt-human",
            Some((Level::High, Level::Xhigh)),
            "gpt-human (high)",
        ),
    ] {
        let mut fixture = seat(COMMAND, "");
        fixture.observed = pair(model, Some("gpt-original"), effort);
        assert_eq!(
            observed_hold("main", &fixture),
            Some(ObservedHold::Standing {
                shown: shown.to_owned()
            }),
            "{model} / {effort:?}"
        );
    }
}

#[test]
fn absent_at_pin_stale_and_foreign_records_do_not_hold() {
    let mut fixtures = vec![seat(COMMAND, "")];
    for (model, pin, effort) in [
        ("gpt-original", "gpt-original", None),
        (
            "gpt-original",
            "gpt-original",
            Some((Level::Xhigh, Level::Xhigh)),
        ),
        ("gpt-human", "gpt-old", None),
        ("gpt-human", "gpt-original", Some((Level::High, Level::Max))),
    ] {
        let mut fixture = seat(COMMAND, "");
        fixture.observed = pair(model, Some(pin), effort);
        fixtures.push(fixture);
    }
    let mut foreign_tool = seat(COMMAND, "");
    foreign_tool.recorded_tool = ToolKind::Claude;
    foreign_tool.observed = pair("gpt-human", Some("gpt-original"), None);
    fixtures.push(foreign_tool);
    let mut foreign_label = seat(COMMAND, "");
    foreign_label.observed = pair("Opus 5", Some("gpt-original"), None);
    fixtures.push(foreign_label);
    for fixture in fixtures {
        assert_eq!(
            observed_hold("main", &fixture),
            None,
            "{:?}",
            fixture.observed
        );
    }
}

#[test]
fn report_only_unmapped_and_mixed_stale_unusable_effort_still_hold() {
    for (command, tool, model, pin, effort, why) in [
        (
            "codex",
            ToolKind::Codex,
            "gpt-human",
            None,
            None,
            "no model",
        ),
        (
            "claude --model opus --effort xhigh",
            ToolKind::Claude,
            "Sonnet 5.5",
            Some("opus"),
            None,
            "no profile",
        ),
        (
            "codex -m gpt-original -c model_reasoning_effort=bogus",
            ToolKind::Codex,
            "gpt-human",
            Some("gpt-old"),
            Some((Level::High, Level::Xhigh)),
            "effort setting is unusable",
        ),
    ] {
        let mut fixture = seat(command, "");
        fixture.tool = tool;
        fixture.recorded_tool = tool;
        fixture.observed = pair(model, pin, effort);
        let Some(ObservedHold::ReportOnly { shown, why: actual }) = observed_hold("main", &fixture)
        else {
            panic!("report-only must hold: {command}");
        };
        assert!(
            shown.starts_with(model) && actual.contains(why),
            "{shown}: {actual}"
        );
    }
}

#[test]
fn damage_is_uncertainty_and_never_claimed_as_a_valid_choice() {
    for rows in [
        "observed_effort.main=high\nobserved_effort_pin.main=xhigh\n",
        "observed_model.main=gpt-human\nobserved_model.main=gpt-other\n",
        "observed_model.main=gpt-human\nobserved_effort.main=high\n",
        "observed_model.main=\n",
    ] {
        let fixture = seat(COMMAND, rows);
        assert!(
            matches!(fixture.observed, Rows::Damaged(_)),
            "fixture: {rows}"
        );
        assert!(
            matches!(
                observed_hold("main", &fixture),
                Some(ObservedHold::Damaged(_))
            ),
            "{rows}"
        );
    }
}
