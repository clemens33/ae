//! #287 slice2: rendered configured evidence, narrow rows and compact navigation.
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::{Hit, Layout, Paint, paint_settings, settings_body};
use crate::app::model::SettingsTab;
use crate::app::settings::{SettingsBodies, StatsScope};
use crate::console::input::{Mouse, MouseKind};
use crate::stats::{self, Reason, Source};
use crate::stats_fixture as f;

fn bodies(this: Option<&str>) -> SettingsBodies {
    let (s, t) = f::matrix();
    let view = stats::fold(
        vec![
            Source::live("S", format!("session_id={}\n", f::S).as_bytes(), Ok(s)),
            Source::live("T", format!("session_id={}\n", f::T).as_bytes(), Ok(t)),
        ],
        this,
    );
    let mut bodies = SettingsBodies::default();
    bodies.stats.view = Some(Box::new(view));
    bodies
}
fn lines(bodies: &SettingsBodies, width: u16) -> Vec<String> {
    settings_body(
        SettingsTab::Statistics,
        bodies,
        Paint::of(None),
        width,
        false,
    )
    .into_iter()
    .map(|(text, _)| text)
    .collect()
}
fn painted(bodies: &SettingsBodies, width: u16, height: u16, scroll: usize) -> (String, Layout) {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let mut layout = Layout::default();
    paint_settings(
        SettingsTab::Statistics,
        scroll,
        bodies,
        None,
        &mut buf,
        &mut layout,
    );
    let text = (0..height)
        .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    (text, layout)
}

#[test]
fn statistics_header_explains_configured_evidence_ranges_gaps_and_retention() {
    let feed = bodies(Some(f::S));
    let rows = lines(&feed, 180);
    let text = rows.join("\n");
    for word in [
        "All retained",
        "Recorded worker spawns: 12",
        "2026-10-01",
        "sources 2 / 2",
        "Evidence: configured launch selection, resolved at spawn",
        "Configured model: 7 / 12",
        "effort: 7 / 12",
        "Partial",
        "certificate lost",
        "ambiguous-model",
        "unknown-effort",
        "none set is no flag, not no model",
        "direct",
        "[clients]",
        "route, not an account",
        "purge",
        "archive's counts at once",
        "from a store kept up to date",
        "Legacy spawns: 2",
        "lower bound",
    ] {
        assert!(text.contains(word), "missing {word}: {text}");
    }
    let positions: Vec<_> = ["Harness", "Client", "Model + effort", "Joint"]
        .into_iter()
        .map(|heading| {
            rows.iter()
                .position(|row| row.trim() == heading)
                .unwrap_or_else(|| panic!("missing table heading {heading}: {text}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "stacked tables in issue order"
    );
    assert!(!text.contains(f::PRIVATE));
    assert!(
        !text.contains("preset"),
        "profiles never become displayed grouping keys"
    );
    assert!(
        !text.contains("Observed initial"),
        "observed selector belongs to a later slice"
    );
}

#[test]
fn this_session_switches_the_same_view_and_no_uuid_never_shows_all_as_current() {
    let mut feed = bodies(Some(f::S));
    feed.stats.scope = StatsScope::ThisSession;
    let text = lines(&feed, 180).join("\n");
    assert!(text.contains("This session"));
    assert!(text.contains("Recorded worker spawns: 10"));
    assert!(text.contains("Configured model: 5 / 10"));
    assert!(text.contains("effort: 5 / 10"));
    assert!(!text.contains("Recorded worker spawns: 12"));
    let mut no_uuid = bodies(None);
    no_uuid.stats.scope = StatsScope::ThisSession;
    let text = lines(&no_uuid, 100).join("\n");
    assert!(text.contains("no session id"), "{text}");
    assert!(
        !text.contains("Recorded worker spawns: 12"),
        "unavailable scope cannot borrow all retained"
    );
    let loading = lines(&SettingsBodies::default(), 100).join("\n");
    assert!(loading.contains("loading"));
    assert!(
        !loading.contains("Recorded worker spawns: 0"),
        "unread is not zero"
    );
}

#[test]
fn narrow_tables_retain_every_unknown_row_count_and_total_and_scroll_to_the_end() {
    let feed = bodies(Some(f::S));
    let rows = lines(&feed, 40);
    let text = rows.join("\n");
    assert!(text.contains("Recorded worker spawns: 12"), "{text}");
    let unknowns: Vec<_> = rows
        .iter()
        .filter(|line| line.contains("unknown"))
        .collect();
    assert!(
        unknowns.len() >= 4,
        "unknown belongs in each of four tables: {text}"
    );
    assert!(
        unknowns
            .iter()
            .filter(|line| line.split_whitespace().last() == Some("3"))
            .count()
            >= 4,
        "unknown count survives clipping in each table: {text}"
    );
    let (top, _) = painted(&feed, 40, 8, 0);
    let (bottom, _) = painted(&feed, 40, 8, usize::MAX);
    assert_ne!(top, bottom, "stacked body scrolls");
}

#[test]
fn long_table_labels_fit_the_panel_and_leave_their_counts_visible() {
    let long = f::fields(
        "codex",
        Some(&"c".repeat(100)),
        Some(&"m".repeat(100)),
        Some("high"),
    );
    let mut bytes = String::new();
    for birth in 1..=11 {
        bytes.push_str(&f::birth(f::S, birth, &long));
    }
    bytes.push_str(&f::birth(
        f::S,
        12,
        &f::fields("codex", None, Some("sol"), Some("high")),
    ));
    let mut feed = SettingsBodies::default();
    feed.stats.view = Some(Box::new(stats::fold(
        vec![Source::archive(f::S, Ok(bytes.into_bytes()))],
        Some(f::S),
    )));
    for width in [40, 41, 57, 80, 180] {
        let rows = lines(&feed, width);
        let (screen, _) = painted(&feed, width, 100, 0);
        let painted_rows: Vec<_> = screen.lines().collect();
        for (heading, counts) in [
            ("Harness", vec!["12"]),
            ("Client", vec!["11", "1"]),
            ("Model + effort", vec!["11", "1"]),
            ("Joint", vec!["11", "1"]),
        ] {
            let start = rows.iter().position(|row| row == heading).expect("table");
            let table: Vec<_> = rows[start + 1..]
                .iter()
                .take_while(|row| !row.is_empty())
                .collect();
            assert_eq!(table.len(), counts.len(), "{heading}, width {width}");
            let painted_start = painted_rows
                .iter()
                .position(|row| row.trim() == heading)
                .expect("table heading painted");
            for (index, (row, count)) in table.iter().zip(counts).enumerate() {
                // These fixtures use ASCII labels and one-cell join/ellipsis marks.
                assert!(
                    row.chars().count() <= usize::from(width - 2),
                    "{heading} exceeds panel room at width {width}: {row:?}"
                );
                assert_eq!(row.split_whitespace().last(), Some(count));
                assert_eq!(
                    painted_rows[painted_start + 1 + index]
                        .split_whitespace()
                        .last(),
                    Some(count),
                    "{heading} count clipped at width {width}"
                );
            }
        }
    }
}

#[test]
fn minimum_panel_keeps_all_seven_compact_titles_click_targets_and_hint() {
    let feed = bodies(Some(f::S));
    let (screen, layout) = painted(&feed, 40, 8, 0);
    let title_row = screen.lines().nth(1).expect("tabs");
    for title in ["Quota", "Conf", "About", "Instr", "Keys", "Cli", "Stats"] {
        assert!(title_row.contains(title), "{title}: {screen}");
    }
    assert!(screen.contains("Settings"));
    assert!(screen.contains("Esc close"), "{screen}");
    for tab in [
        SettingsTab::Quota,
        SettingsTab::Config,
        SettingsTab::About,
        SettingsTab::Instructions,
        SettingsTab::Keys,
        SettingsTab::Accounts,
        SettingsTab::Statistics,
    ] {
        assert!((0..40).any(|column| matches!(layout.hit(Mouse { kind: MouseKind::Click, column, row:1 }), Some(Hit::SettingsTab(hit)) if hit == tab)), "every compact tab is clickable: {tab:?}");
    }
    let (wide, _) = painted(&feed, 180, 50, 0);
    assert!(
        wide.lines()
            .nth(1)
            .expect("wide tabs")
            .contains("Statistics")
    );
    let help = settings_body(SettingsTab::Keys, &feed, Paint::of(None), 180, false);
    assert!(
        help.iter().any(|(line, _)| line.contains("1-7")),
        "tab help names new digit"
    );
}

#[test]
fn configured_fraction_labels_do_not_borrow_the_other_dimension() {
    let bytes = f::birth(
        f::S,
        1,
        &format!(
            "{},\"birth_effort_unproved\":\"unknown-effort\"",
            f::fields("codex", None, Some("sol"), None)
        ),
    );
    let mut feed = SettingsBodies::default();
    feed.stats.view = Some(Box::new(stats::fold(
        vec![Source::archive(f::S, Ok(bytes.into_bytes()))],
        Some(f::S),
    )));
    let text = lines(&feed, 180).join("\n");
    assert!(text.contains("Configured model: 1 / 1"), "{text}");
    assert!(text.contains("effort: 0 / 1"), "{text}");
    assert!(text.contains("unknown-effort"));
}

#[test]
fn supported_empty_history_says_zero_and_complete_but_source_failure_stays_partial() {
    let mut feed = SettingsBodies::default();
    feed.stats.view = Some(Box::new(stats::fold(
        vec![Source::archive(f::S, Ok(Vec::new()))],
        Some(f::S),
    )));
    let empty = lines(&feed, 180).join("\n");
    assert!(empty.contains("Recorded worker spawns: 0"));
    assert!(empty.contains("Coverage: complete"), "{empty}");
    feed.stats.view = Some(Box::new(stats::fold(
        vec![Source::failed(
            "missing-source",
            Some(f::S),
            crate::stats::SourceGap::Missing,
        )],
        Some(f::S),
    )));
    let missing = lines(&feed, 180).join("\n");
    assert!(missing.contains("Partial"));
    assert!(missing.contains("missing-source"));
    assert!(
        !missing.contains("Coverage: complete"),
        "a gap cannot render a healthy empty history"
    );
}

#[test]
fn unproved_flag_and_live_seat_notes_keep_fully_read_coverage_complete() {
    let cert = f::birth(
        f::S,
        1,
        &format!(
            "{},\"birth_model_unproved\":\"unsupported\",\"birth_effort_unproved\":\"unsupported\"",
            f::fields("muse", None, None, None)
        ),
    );
    let meta = format!("session_id={}\nseat.spawned.7=unproved-seat\n", f::S);
    let view = stats::fold(
        vec![Source::live("S", meta.as_bytes(), Ok(cert.into_bytes()))],
        Some(f::S),
    );
    assert_eq!(view.all.total, 1, "live seat note adds no birth");
    assert_eq!((view.all.sources_read, view.all.sources_total), (1, 1));
    assert_eq!(
        view.all.gaps.len(),
        3,
        "two flag notes and one live seat note"
    );
    for model in [false, true] {
        assert!(view.all.gaps.iter().any(|gap| gap.reason
            == Reason::Unproved {
                model,
                word: "unsupported".to_owned()
            }
            && gap.count == 1));
    }
    let live_note = view
        .all
        .gaps
        .iter()
        .find(|gap| gap.reason == Reason::LiveNoBirth)
        .expect("unproved live seat is a visible note");
    assert_eq!(live_note.count, 1);
    let note_lines: Vec<_> = view.all.gaps.iter().map(crate::stats::Gap::line).collect();
    assert!(
        !view.all.partial(),
        "notes alone do not mean partial evidence"
    );
    let mut feed = SettingsBodies::default();
    feed.stats.view = Some(Box::new(view));
    let text = lines(&feed, 180).join("\n");
    assert!(text.contains("Recorded worker spawns: 1"), "{text}");
    assert!(text.contains("Coverage: complete"), "{text}");
    assert!(text.contains("unsupported"), "{text}");
    assert!(
        !text.contains("Partial"),
        "notes must not imply lost evidence: {text}"
    );
    for note in note_lines {
        assert!(
            text.contains(&note),
            "coverage note remains visible: {note}: {text}"
        );
    }
}

#[test]
fn legacy_lower_bound_alone_keeps_fully_read_coverage_complete() {
    let view = stats::fold(
        vec![Source::archive(
            f::S,
            Ok(f::legacy(f::PRIVATE).into_bytes()),
        )],
        Some(f::S),
    );
    assert_eq!((view.all.total, view.all.legacy), (1, 1));
    assert_eq!((view.all.sources_read, view.all.sources_total), (1, 1));
    assert!(!view.all.partial());
    let mut feed = SettingsBodies::default();
    feed.stats.view = Some(Box::new(view));
    let text = lines(&feed, 180).join("\n");
    assert!(text.contains("Coverage: complete"), "{text}");
    assert!(text.contains("Legacy spawns: 1"), "{text}");
    assert!(text.contains("lower bound"), "{text}");
    assert!(!text.contains("Partial"), "{text}");
    assert!(!text.contains(f::PRIVATE));
}
