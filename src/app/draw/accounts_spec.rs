//! Frozen acceptance: dedicated Accounts Settings tab (#242 s3 / human b717ce77).
//! Oracles: .local/seatview/nav-surface-map.md, rulings 8a9221f2 + 2c127c84,
//! .local/gate-rules.md ACTIVE; busy = working tone (never `needs_you`, colead).

use super::{Hit, LEFT, Layout, Paint, paint_settings, settings_body};
use crate::app::fleet::{Counts, Fleet, Line2, Row};
use crate::app::model::{Key, Model, SettingsTab};
use crate::app::settings::SettingsBodies;
use crate::console::input::{Mouse, MouseKind};
use crate::quota::{LiveGroup, LiveRow, LiveSeats, view};
use crate::theme::Look;
use crate::theme::Mark;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};

const SECTION: &str = "Live seats by account";
const KEPT: &str = "quota windows held 2m ago";

fn look() -> Look {
    Look::read("on", "darcula", "on", "on")
}

fn window() -> view::Window {
    view::Window {
        name: "Weekly".into(),
        raw: Some("31%".into()),
        counts: None,
        fill: Some(2),
        why: None,
        level: view::Level::Headroom,
        stale: false,
        resets: Some("2d".into()),
        seen: Some("1m".into()),
        burn: None,
    }
}

fn row(
    session_seat: &str,
    profile: &str,
    model: &str,
    effort: &str,
    activity: &str,
    declared: &str,
) -> LiveRow {
    LiveRow {
        session_seat: session_seat.to_owned(),
        profile: profile.to_owned(),
        model: model.to_owned(),
        effort: effort.to_owned(),
        activity: activity.to_owned(),
        declared: declared.to_owned(),
    }
}

fn seats() -> LiveSeats {
    LiveSeats {
        incomplete: false,
        groups: vec![
            LiveGroup {
                key: None,
                heading: "claude · /acct/.claude.json".into(),
                rows: vec![
                    row("s:lead", "fablex", "Opus 5.5", "xhigh", "busy", "done"),
                    row("tw:b", "sol61x", "spec", "max", "idle", "working"),
                ],
                ..LiveGroup::default()
            },
            LiveGroup {
                key: None,
                heading: "Account unknown".into(),
                gap: Some("home not recorded".into()),
                rows: vec![row("gh:one", "宽宽宽", "glm", "zen", "busy", "?")],
                ..LiveGroup::default()
            },
        ],
    }
}

fn bodies(live_seats: Option<LiveSeats>) -> SettingsBodies {
    SettingsBodies {
        quota: Some(vec![view::Scope {
            title: "claude · cc".into(),
            profiles: "fablex".into(),
            account: None,
            only: None,
            windows: vec![window()],
        }]),
        quota_kept: Some(KEPT.to_owned()),
        live_seats,
        ..SettingsBodies::default()
    }
}

fn inked(tab: SettingsTab, width: u16, feed: &SettingsBodies, ink: &Paint) -> Vec<(String, Style)> {
    settings_body(tab, feed, *ink, width, true)
}

fn click_hit(width: u16, want: SettingsTab) -> Option<Hit> {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, 20));
    let mut layout = Layout::default();
    paint_settings(
        SettingsTab::Quota,
        0,
        &bodies(Some(seats())),
        Some(&look()),
        &mut buf,
        &mut layout,
    );
    (0..width)
        .flat_map(|column| {
            (0..3u16).map(move |row| Mouse {
                kind: MouseKind::Click,
                column,
                row,
            })
        })
        .find_map(|mouse| {
            (matches!(layout.hit(mouse), Some(Hit::SettingsTab(t)) if t == want))
                .then(|| Hit::SettingsTab(want))
        })
}

fn nav_fleet() -> Fleet {
    Fleet {
        rows: vec![Row {
            name: "api".to_owned(),
            index: 1,
            mark: Mark::Working,
            needy: false,
            unanswered: None,
            counts: Counts::Unknown,
            line2: Line2::NoGoal,
            home: false,
        }],
        home: None,
    }
}

/// Wrong: seat content left on Quota, quota windows copied to Accounts, or a
/// body that concatenates the tab contents to preserve obsolete same-body
/// ordering.
#[test]
fn accounts_shows_seats_and_quota_keeps_only_windows() {
    let ink = Paint::of(Some(&look()));
    let feed = bodies(Some(seats()));
    let acc = inked(SettingsTab::Accounts, 160, &feed, &ink);
    assert!(acc.iter().any(|(t, _)| t.contains(SECTION)), "{acc:?}");
    assert!(acc.iter().any(|(t, _)| t.contains("s:lead")), "{acc:?}");
    assert!(
        acc.iter().any(|(t, _)| t.contains("Account unknown")),
        "{acc:?}"
    );
    assert!(!acc.iter().any(|(t, _)| t.contains("Weekly")), "{acc:?}");
    let quota = inked(SettingsTab::Quota, 160, &feed, &ink);
    assert!(quota.iter().any(|(t, _)| t.contains("Weekly")), "{quota:?}");
    assert!(!quota.iter().any(|(t, _)| t.contains(SECTION)), "{quota:?}");
    assert!(
        !quota.iter().any(|(t, _)| t.contains("s:lead")),
        "{quota:?}"
    );
}

/// Wrong: digit 6 swallowed, 1-5 renumbered, generation lost on a tab change.
#[test]
fn digit_six_and_tab_cycle_reach_accounts_keeping_one_through_five() {
    let fl = nav_fleet();
    for (digit, tab) in [
        (1, SettingsTab::Quota),
        (2, SettingsTab::Config),
        (3, SettingsTab::About),
        (4, SettingsTab::Instructions),
        (5, SettingsTab::Keys),
    ] {
        let mut m = Model::new(&fl);
        m.open_settings(9);
        let _ = m.key(Key::Digit(digit), &fl, false);
        assert_eq!(m.settings().expect("open").tab, tab, "digit {digit}");
        assert!(m.settings_generation(9), "generation pinned on {digit}");
    }
    let mut m = Model::new(&fl);
    m.open_settings(7);
    let _ = m.key(Key::Digit(6), &fl, false);
    assert_eq!(
        m.settings().expect("open").tab,
        SettingsTab::Accounts,
        "digit 6"
    );
    assert!(m.settings_generation(7), "generation pinned across switch");
    for _ in 0..5 {
        let _ = m.key(Key::Tab, &fl, false);
    }
    assert_eq!(
        m.settings().expect("open").tab,
        SettingsTab::Keys,
        "cycle wraps"
    );
}

/// Wrong: no Accounts click target, or one that leaves the drawn grid.
#[test]
fn mouse_accounts_title_click_selects_the_tab() {
    let wide = click_hit(120, SettingsTab::Accounts);
    assert!(wide.is_some(), "scaffold: Accounts title not rendered");
    let narrow = click_hit(40, SettingsTab::Accounts);
    assert!(narrow.is_some(), "Accounts click target at 40 cells");
}

/// Wrong: the empty/loading/unknown/kept spellings do not survive the move.
#[test]
fn empty_loading_and_kept_stay_honest_on_accounts() {
    let ink = Paint::of(Some(&look()));
    let loading = inked(SettingsTab::Accounts, 80, &bodies(None), &ink);
    let empty = inked(
        SettingsTab::Accounts,
        80,
        &bodies(Some(LiveSeats::default())),
        &ink,
    );
    assert!(
        loading.len() >= 2
            && loading.iter().any(|(t, _)| t.contains("loading"))
            && empty.iter().any(|(t, _)| t.contains("no live seats")),
        "{loading:?} {empty:?}"
    );
    let feed = bodies(Some(seats()));
    let acc = inked(SettingsTab::Accounts, 160, &feed, &ink);
    assert!(acc[0].0.contains(KEPT), "kept line leads the body: {acc:?}");
    assert!(
        acc.iter().any(|(t, _)| t.contains("home not recorded")),
        "{acc:?}"
    );
}

/// (N1: header dropped at 40, honest clips, identity first); cells by CJK.
fn cell_w(ch: char) -> usize {
    1 + usize::from(('\u{4e00}'..='\u{9fff}').contains(&ch))
}

#[test]
fn wide_rows_align_columns_and_narrow_clips_honestly() {
    let ink = Paint::of(Some(&look()));
    let feed = bodies(Some(seats()));
    let got = inked(SettingsTab::Accounts, 160, &feed, &ink);
    let at = got
        .iter()
        .position(|(t, _)| t.contains(SECTION))
        .expect("section row")
        + 1;
    // Header row: every label once, in draw order; its starts are the oracle.
    let header = &got[at].0;
    let mid = inked(SettingsTab::Accounts, 60, &feed, &ink);
    let mut from = 0;
    let starts: Vec<usize> = [
        "session", "profile", "model", "effort", "activity", "declared",
    ]
    .iter()
    .map(|label| {
        let at = header[from..].find(label).expect("label in header") + from;
        from = at + 1;
        at
    })
    .collect();
    // Value cells share the label starts, per row, across groups.
    for (values, row) in [
        (
            [
                "s:lead",
                "fablex",
                "Opus 5.5",
                "xhigh",
                "activity=",
                "declared=",
            ],
            "s:lead",
        ),
        (
            ["tw:b", "sol61x", "spec", "max", "activity=", "declared="],
            "tw:b",
        ),
        (
            ["gh:one", "宽宽宽", "glm", "zen", "activity=", "declared="],
            "gh:one",
        ),
    ] {
        let t = got
            .iter()
            .find(|(t, _)| t.contains(row))
            .expect("row present")
            .0
            .clone();
        for (needle, head) in values.into_iter().zip(starts.iter()) {
            // Terminal-CELL index of the needle in the row: a CJK value must
            // not shift a later column's start (I2).
            let found = t.find(needle).expect("cell present");
            let cell_at = t[..found].chars().map(cell_w).sum::<usize>();
            assert_eq!(cell_at, *head, "row {row} {needle}");
        }
    }
    // 60-col cut (NIT/review): identity + omission, no fixed trailing chars.
    let busy = mid
        .iter()
        .find(|(t, _)| t.contains("activity=busy"))
        .expect("busy row")
        .0
        .clone();
    let cells: usize = busy.chars().map(cell_w).sum();
    assert!(
        cells <= usize::from(60 - LEFT)
            && busy.contains("s:lead")
            && busy.trim_end().ends_with('…'),
        "busy row fits, keeps identity, marks omission: {busy:?}"
    );
    let narrow = inked(SettingsTab::Accounts, 40, &feed, &ink);
    let budget = usize::from(40 - LEFT);
    for (t, _) in &narrow {
        let cells: usize = t.chars().map(cell_w).sum();
        assert!(cells <= budget, "{cells} cells: {t:?}");
        assert!(!t.contains('\u{1b}'), "neutral: {t:?}");
    }
    assert!(
        narrow.iter().any(|(t, _)| t.contains("s:lead")),
        "{narrow:?}"
    );
    assert!(
        !narrow.iter().any(|(t, _)| t.contains("profile")),
        "{narrow:?}"
    );
    assert!(
        narrow.iter().any(|(t, _)| t.contains('…')),
        "omission: {narrow:?}"
    );
    print_evidence();
}

/// Wrong: a declared difference recolors a row of equal observed activity.
#[test]
fn row_tones_follow_observed_activity_and_retained_state() {
    let ink = Paint::of(Some(&look()));
    let feed = bodies(Some(seats()));
    let rows = inked(SettingsTab::Accounts, 160, &feed, &ink);
    let tone_of = |needle: &str| {
        rows.iter()
            .find(|(t, _)| t.contains(needle))
            .map(|(_, s)| *s)
            .expect(needle)
    };
    let working = ink.fg(|p| p.working);
    assert_eq!(
        tone_of("activity=busy  declared=done"),
        working,
        "busy tone"
    );
    assert_eq!(tone_of("activity=idle"), ink.fg(|p| p.text), "idle plain");
    let head = ink.fg(|p| p.text).add_modifier(Modifier::BOLD);
    assert_eq!(tone_of(SECTION), head, "section bold");
    assert_eq!(tone_of("Account unknown"), head, "group heading bold");
    assert_eq!(
        tone_of("home not recorded"),
        ink.fg(|p| p.stale),
        "gap stale"
    );
    assert_eq!(tone_of(KEPT), ink.fg(|p| p.stale), "retained stale");
    assert_eq!(
        tone_of("gh:one"),
        working,
        "equal observed activity, distinct declared text"
    );
    assert_ne!(working, ink.fg(|p| p.dim), "palette sanity");
    assert_ne!(working, ink.fg(|p| p.stale), "palette sanity");
}

/// Render evidence via nextest --success-output immediate; not a behavior pin.
fn print_evidence() {
    for (name, look) in [("theme-on", Some(look())), ("theme-off", None)] {
        for tab in [SettingsTab::Accounts, SettingsTab::Quota] {
            for width in [160u16, 80, 40] {
                let mut buf = Buffer::empty(Rect::new(0, 0, width, 24));
                let mut layout = Layout::default();
                paint_settings(
                    tab,
                    0,
                    &bodies(Some(seats())),
                    look.as_ref(),
                    &mut buf,
                    &mut layout,
                );
                println!("== {name} {tab:?} width {width}");
                for y in 0..24u16 {
                    let mut line = String::new();
                    for x in 0..width {
                        line.push_str(buf[(x, y)].symbol());
                    }
                    println!("row {y} style {:?} | {}", buf[(LEFT, y)].style(), line);
                }
            }
        }
    }
}
