//! Acceptance oracle: approved design-r3 sections 4–5, before implementation.
//! `task` remains declaration text; owner ruling 2e5aaaa8 adds `background_task`.
#![allow(
    clippy::expect_used,
    reason = "acceptance fixtures require complete parsed facts"
)]

use ae::digest::{AgentEntry, SessionEntry, Status};
use ae::json::Value;
use ae::theme::{Mark, Palette};
use ae::time::Timestamp;
use ae::tmux::parse_picker_agents;
use std::collections::BTreeMap;
use std::fmt::Write as _;

const NOW: i64 = 2_000;

fn bgmark_agent(state: &str, task: bool) -> ae::tmux::PickerAgent {
    let raw = format!(
        "v3;2000;60;lead:p:{state}:%3:claude::::{}",
        if task { "1" } else { "" }
    );
    parse_picker_agents(&raw, NOW)
        .expect("valid v3 fixture")
        .remove(0)
}

#[test]
fn bgmark_glyph_word_rank_and_order_are_the_approved_vocabulary() {
    assert_eq!(Mark::Background.glyph(true), "◉");
    assert_eq!(Mark::Background.glyph(false), "&");
    assert_eq!(Mark::Background.word(), "background");
    assert_eq!(Mark::Background.rank(), 2);
    assert_eq!(Mark::from_rank("2"), Mark::Working);
    assert_eq!(
        Mark::BY_URGENCY.as_slice(),
        [
            Mark::Dead,
            Mark::NeedsYou,
            Mark::Stale,
            Mark::Working,
            Mark::Background,
            Mark::WaitingAgent,
            Mark::Done,
            Mark::Idle
        ]
    );
    for marks in [
        [Mark::WaitingAgent, Mark::Background],
        [Mark::Background, Mark::WaitingAgent],
    ] {
        assert_eq!(Mark::most_actionable(marks), Mark::Background);
    }
}

#[test]
fn bgmark_shared_composition_masks_only_the_three_quiet_marks() {
    for (base, expected) in [
        (Mark::Dead, Mark::Dead),
        (Mark::NeedsYou, Mark::NeedsYou),
        (Mark::Stale, Mark::Stale),
        (Mark::Working, Mark::Working),
        (Mark::WaitingAgent, Mark::Background),
        (Mark::Done, Mark::Background),
        (Mark::Idle, Mark::Background),
        (Mark::Background, Mark::Background),
    ] {
        assert_eq!(base.with_task(false), base);
        assert_eq!(base.with_task(true), expected);
    }
}

#[test]
fn bgmark_v3_accepts_only_exact_arity_and_empty_or_one_task_cell() {
    for task in ["", "1"] {
        let raw = format!("v3;2000;60;lead:p:waiting-agent:%3:claude::::{task}");
        let agents = parse_picker_agents(&raw, NOW).expect("approved task spelling");
        assert_eq!(agents[0].task, task == "1");
        assert_eq!(agents[0].state, "waiting-agent");
        assert_eq!(
            agents[0].mark(),
            if task == "1" {
                Mark::Background
            } else {
                Mark::WaitingAgent
            }
        );
    }
    for entry in [
        "lead:p:waiting-agent:%3:claude:::",
        "lead:p:waiting-agent:%3:claude::::1:",
        "lead:p:waiting-agent:%3:claude::::0",
        "lead:p:waiting-agent:%3:claude::::true",
        "lead:p:waiting-agent:%3:claude::::!",
        "lead:p:waiting-agent:%3:claude::::11",
        "lead:p:waiting-agent:%3:claude:::: 1",
    ] {
        assert!(
            parse_picker_agents(&format!("v3;2000;60;{entry}"), NOW).is_none(),
            "{entry}"
        );
        let mixed = format!("v3;2000;60;other:p:done:%4:claude::::;{entry}");
        assert!(
            parse_picker_agents(&mixed, NOW).is_none(),
            "reject whole snapshot: {mixed}"
        );
    }
}

#[test]
fn bgmark_old_grammars_never_claim_a_task_and_picker_composes_every_state() {
    for raw in [
        "v1;2000;60;lead:p:waiting-agent:%3",
        "v2;2000;60;lead:p:waiting-agent:%3:claude:::",
    ] {
        let agent = parse_picker_agents(raw, NOW)
            .expect("legacy grammar")
            .remove(0);
        assert!(!agent.task);
        assert_eq!(agent.mark(), Mark::WaitingAgent);
    }
    for (state, base) in [
        ("waiting-agent", Mark::WaitingAgent),
        ("done", Mark::Done),
        ("idle", Mark::Idle),
        ("working", Mark::Working),
        ("sweeping", Mark::Working),
        ("busy", Mark::Working),
        ("waiting-user", Mark::NeedsYou),
        ("blocked", Mark::NeedsYou),
        ("throttled", Mark::NeedsYou),
        ("limit", Mark::NeedsYou),
        ("prompt", Mark::NeedsYou),
        ("wedged", Mark::NeedsYou),
        ("stale", Mark::Stale),
        ("unreachable", Mark::Stale),
        ("starting", Mark::Stale),
        ("dead", Mark::Dead),
    ] {
        assert_eq!(bgmark_agent(state, false).mark(), base, "{state}");
        let expected = if matches!(state, "waiting-agent" | "done" | "idle") {
            Mark::Background
        } else {
            base
        };
        assert_eq!(bgmark_agent(state, true).mark(), expected, "{state}");
    }
}

fn bgmark_luminance(colour: &str) -> f64 {
    let component = |i| {
        let value =
            f64::from(u8::from_str_radix(&colour[i..i + 2], 16).expect("hex token")) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * component(1) + 0.7152 * component(3) + 0.0722 * component(5)
}

#[test]
fn bgmark_accent_is_distinct_from_each_old_mark_and_clears_the_bar() {
    for palette in [Palette::DARCULA, Palette::NEUTRAL, Palette::WARM] {
        let accent = palette.accent(Mark::Background);
        for old in [
            Mark::Dead,
            Mark::NeedsYou,
            Mark::Stale,
            Mark::Working,
            Mark::WaitingAgent,
            Mark::Done,
            Mark::Idle,
        ] {
            assert!(
                !accent.eq_ignore_ascii_case(palette.accent(old)),
                "{} {old:?}",
                palette.name
            );
        }
        let a = bgmark_luminance(accent);
        let b = bgmark_luminance(palette.base);
        let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        assert!(ratio >= 3.0, "{} {accent}: {ratio}", palette.name);
    }
}

#[test]
fn bgmark_fleet_row_and_tally_keep_background_ahead_of_other_quiet_marks() {
    use ae::app::fleet::{Counts, Facts};
    let now = Timestamp::from_epoch(NOW);
    for reverse in [false, true] {
        let mut agents = vec![
            bgmark_agent("waiting-agent", true),
            bgmark_agent("done", false),
        ];
        agents[1].name = "other".to_owned();
        if reverse {
            agents.reverse();
        }
        let facts = BTreeMap::from([(
            "s".to_owned(),
            Facts::Seats {
                id: "$1".to_owned(),
                agents,
            },
        )]);
        let world = ae::listing::World::new(now, vec![SessionEntry::new("s", Status::Running)]);
        let fleet = ae::app::fleet::rows(
            &world,
            &facts,
            &BTreeMap::new(),
            &ae::theme::FleetOrder::EMPTY,
            &BTreeMap::new(),
            None,
            now,
        );
        assert_eq!(fleet.rows[0].mark, Mark::Background);
        assert_eq!(
            fleet.rows[0].counts,
            Counts::Marks(vec![(Mark::Background, false, 1), (Mark::Done, false, 1)])
        );
    }
    let mut active = bgmark_agent("working", false);
    active.name = "active".to_owned();
    let facts = BTreeMap::from([(
        "s".to_owned(),
        Facts::Seats {
            id: "$1".to_owned(),
            agents: vec![bgmark_agent("idle", true), active],
        },
    )]);
    let world = ae::listing::World::new(now, vec![SessionEntry::new("s", Status::Running)]);
    let fleet = ae::app::fleet::rows(
        &world,
        &facts,
        &BTreeMap::new(),
        &ae::theme::FleetOrder::EMPTY,
        &BTreeMap::new(),
        None,
        now,
    );
    assert_eq!(
        fleet.rows[0].counts,
        Counts::Marks(vec![
            (Mark::Working, false, 1),
            (Mark::Background, false, 1)
        ])
    );
}

fn bgmark_entry() -> SessionEntry {
    let mut session = SessionEntry::new("s", Status::Running);
    session.agents = vec![AgentEntry {
        reference: "lead".to_owned(),
        name: "lead".to_owned(),
        alias: "p".to_owned(),
        state: Some("waiting-agent".to_owned()),
        alive: Some(true),
        declaration: Some(ae::digest::Declaration::Made {
            reason: "declared words stay intact".to_owned(),
            at: Some(NOW),
            on: None,
            overdue: false,
            missing: false,
            moved: None,
        }),
        ..AgentEntry::default()
    }];
    session
}

#[test]
fn bgmark_list_fact_provenance_false_fallback_and_table_suffix() {
    let meta =
        ae::meta::Meta::parse("schema=2\nlayout=lead-pair\nseat.main=lead\nprofile.main=p\n");
    let roster = meta.roster();
    let pane = ae::tmux::WatchPane {
        pane_id: "%3".to_owned(),
        slot: Some("main".to_owned()),
        agent: Some("lead".to_owned()),
        current_command: String::new(),
        pane_pid: None,
        observed: String::new(),
    };
    for (fact, panes, expected) in [
        (
            Some("v3;2000;60;lead:p:waiting-agent:%3:claude::::1"),
            Some(vec![pane.clone()]),
            true,
        ),
        (
            Some("v3;2000;60;lead:p:working:%3:claude::::1"),
            Some(vec![pane.clone()]),
            true,
        ),
        (
            Some("v3;2000;60;lead:p:waiting-agent:%3:claude::::"),
            Some(vec![pane.clone()]),
            false,
        ),
        (
            Some("v2;2000;60;lead:p:waiting-agent:%3:claude:::"),
            Some(vec![pane.clone()]),
            false,
        ),
        (None, Some(vec![pane.clone()]), false),
        (
            Some("v3;1800;60;lead:p:waiting-agent:%3:claude::::1"),
            Some(vec![pane.clone()]),
            false,
        ),
        (
            Some("v3;2000;60;lead:wrong:waiting-agent:%3:claude::::1"),
            Some(vec![pane.clone()]),
            false,
        ),
        (
            Some("v3;2000;60;lead:p:waiting-agent:%4:claude::::1"),
            Some(vec![pane.clone()]),
            false,
        ),
        (
            Some("v3;2000;60;lead:p:waiting-agent:%3:claude::::1"),
            None,
            false,
        ),
        (
            Some("v3;2000;60;lead:p:waiting-agent:%3:claude::::1"),
            Some(vec![pane.clone(), pane.clone()]),
            false,
        ),
    ] {
        let mut entry = bgmark_entry();
        // Reuse the same entry to prove missing proof clears an earlier true.
        ae::session::enrich_list_json(
            &mut entry,
            roster,
            Some("v3;2000;60;lead:p:waiting-agent:%3:claude::::1"),
            Some(std::slice::from_ref(&pane)),
            Timestamp::from_epoch(NOW),
        );
        ae::session::enrich_list_json(
            &mut entry,
            roster,
            fact,
            panes.as_deref(),
            Timestamp::from_epoch(NOW),
        );
        assert_eq!(entry.agents[0].background_task, expected, "{fact:?}");
        let Value::Arr(agents) = entry.to_json().get("agents").expect("agents field").clone()
        else {
            panic!("agents array")
        };
        assert_eq!(
            agents[0].get("background_task"),
            Some(&Value::Bool(expected))
        );
        assert_eq!(
            agents[0].get("task"),
            Some(&Value::str("declared words stay intact"))
        );
        let table = ae::listing::table_at(&[&entry], Timestamp::from_epoch(NOW));
        assert_eq!(table.contains("+task"), expected, "{table}");
    }
    let mut stopped = bgmark_entry();
    stopped.status = Status::Stopped;
    ae::session::enrich_list_json(
        &mut stopped,
        roster,
        Some("v3;2000;60;lead:p:waiting-agent:%3:claude::::1"),
        Some(&[pane]),
        Timestamp::from_epoch(NOW),
    );
    assert!(!stopped.agents[0].background_task);
}

fn bgmark_format_digest() -> String {
    let mut rendered = String::new();
    for palette in [Palette::DARCULA, Palette::NEUTRAL, Palette::WARM] {
        for icons in [false, true] {
            for drawn in [false, true] {
                for motion in [false, true] {
                    let look = ae::theme::Look {
                        palette,
                        icons,
                        drawn,
                        motion,
                    };
                    let _ = write!(
                        rendered,
                        "{} {icons} {drawn} {motion}\n{:?}\n{:?}\n",
                        palette.name,
                        ae::theme::layout_options(&look),
                        ae::theme::window_options(&look)
                    );
                }
            }
        }
    }
    rendered.push_str(&ae::theme::pane_border_format());
    rendered.push_str(&ae::theme::view_status_format("fixture hint"));
    ae::install::sha256_hex(rendered.as_bytes())
}

#[test]
fn bgmark_format_version_changes_if_and_only_if_rendered_formats_change() {
    let baseline = include_str!("../spec/bgmark-formats-baseline.txt").trim();
    let changed = bgmark_format_digest() != baseline;
    assert_eq!(
        ae::theme::FORMAT_VERSION != "22",
        changed,
        "format version must move exactly when a layout or rendered format moves"
    );
}

#[test]
fn bgmark_v3_fuzz_seeds_reach_both_task_values_and_invalid_cell() {
    for (seed, expected) in [
        (
            include_str!("../../fuzz/seeds/picker_agents/v3-task"),
            Some(true),
        ),
        (
            include_str!("../../fuzz/seeds/picker_agents/v3-no-task"),
            Some(false),
        ),
        (
            include_str!("../../fuzz/seeds/picker_agents/v3-invalid-task"),
            None,
        ),
    ] {
        assert!(
            seed.bytes().all(|b| (b' '..=b'~').contains(&b)),
            "branch must not fail before task: {seed:?}"
        );
        let parsed = parse_picker_agents(seed, 2_000_000_000);
        assert_eq!(parsed.map(|agents| agents[0].task), expected);
    }
}
