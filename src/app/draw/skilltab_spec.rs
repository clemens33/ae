//! Frozen acceptance for the Statistics tab's inner row: seven top tabs
//! again, with one fixed row of inner tabs `Agents`, `Skills`, `Tools`
//! directly under the Settings tab row, tab 7 only, at every width.
//! Authored by navigator `skilltab-nav` for the accepted inner-tabs
//! design before the fix. Install as `src/app/draw/skilltab_spec.rs` and
//! wire from `src/app/draw.rs` under `cfg(test)`. RED on `612a485e` is by
//! assertion only: the row is drawn whole whichever tab is selected, so
//! nothing named here is new on that commit.
#![allow(clippy::expect_used, reason = "frozen acceptance assertions")]

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::{Layout, Paint, paint_settings, settings_body};
use crate::app::model::SettingsTab;
use crate::app::settings::SettingsBodies;
use crate::console::input::{Mouse, MouseKind};

fn painted(width: u16, height: u16) -> (String, Layout) {
    let feed = SettingsBodies::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let mut layout = Layout::default();
    paint_settings(
        SettingsTab::Statistics,
        0,
        &feed,
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

fn tab_row(screen: &str) -> &str {
    screen.lines().nth(1).expect("the tab row").trim_end()
}

fn inner_row(screen: &str) -> &str {
    screen.lines().nth(2).expect("the inner row").trim_end()
}

#[test]
fn top_row_names_the_seven_tabs_in_order() {
    let (screen, _layout) = painted(180, 12);
    let row = tab_row(&screen);
    assert!(!row.contains("Skills & Tools"), "no eighth top tab: {row}");
    let mut next = 0;
    for title in [
        "Quota",
        "Config",
        "About",
        "Instructions",
        "Keys",
        "Clients",
        "Statistics",
    ] {
        let at = row[next..]
            .find(title)
            .unwrap_or_else(|| panic!("{title} missing from the wide tab row: {row}"));
        next += at + title.len();
    }
}

#[test]
fn top_row_at_the_floor_is_the_main_compact_row() {
    let (screen, _layout) = painted(40, 8);
    assert_eq!(tab_row(&screen), "  Quota Conf About Instr Keys Cli Stats");
}

#[test]
fn inner_row_holds_at_every_width() {
    for width in [120, 44, 40] {
        let (screen, _layout) = painted(width, 12);
        let row = inner_row(&screen);
        assert_eq!(
            row, "  Agents  Skills  Tools",
            "the inner row is one fixed form at width {width}"
        );
    }
}

#[test]
fn every_inner_title_is_clickable_at_the_floor() {
    let (screen, layout) = painted(40, 8);
    let mut top: Vec<String> = Vec::new();
    let mut inner: Vec<String> = Vec::new();
    for column in 0..40 {
        if let Some(hit) = layout.hit(Mouse {
            kind: MouseKind::Click,
            column,
            row: 1,
        }) {
            top.push(format!("{hit:?}"));
        }
        if let Some(hit) = layout.hit(Mouse {
            kind: MouseKind::Click,
            column,
            row: 2,
        }) {
            inner.push(format!("{hit:?}"));
        }
    }
    let mut cells: Vec<String> = Vec::new();
    for hit in inner {
        if !cells.contains(&hit) {
            cells.push(hit);
        }
    }
    assert_eq!(
        cells.len(),
        3,
        "each inner title has its own click target: {screen}"
    );
    for hit in &cells {
        assert!(
            !top.contains(hit),
            "inner targets differ from every top title: {screen}"
        );
    }
}

#[test]
fn keys_help_names_digits_1_7() {
    let feed = SettingsBodies::default();
    let help = settings_body(SettingsTab::Keys, &feed, Paint::of(None), 180, false);
    assert!(
        help.iter().any(|(line, _)| line.contains("1-7")),
        "the keys help names digits 1-7"
    );
    assert!(
        !help.iter().any(|(line, _)| line.contains("1-8")),
        "the keys help keeps no 1-8"
    );
    assert!(
        help.iter()
            .any(|(line, _)| line.contains("Statistics") && !line.contains("Skills & Tools")),
        "the m row names Statistics alone"
    );
}
