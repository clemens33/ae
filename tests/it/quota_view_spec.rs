//! Frozen acceptance spec: the shared quota view (quotaview).
//!
//! Oracle: quota-design.md frames + brief D1-D6 + lead GO rulings. Scope
//! literals in, exact lines out; no builder, no tmux, no app here.

use ae::quota::view::{Geometry, Level, Scope, Tone, Why, Window, layout, unavailable};

fn window(name: &str) -> Window {
    Window {
        name: name.to_owned(),
        raw: Some("31%".to_owned()),
        counts: None,
        fill: Some(2),
        why: None,
        level: Level::Headroom,
        stale: false,
        resets: Some("2h10m".to_owned()),
        seen: Some("2m".to_owned()),
        burn: None,
    }
}

fn scope(title: &str, windows: Vec<Window>) -> Scope {
    Scope {
        title: title.to_owned(),
        profiles: String::new(),
        account: None,
        only: None,
        windows,
    }
}

/// The design frame's first three windows: calm, low+stale, derivation.
fn trio() -> Vec<Scope> {
    let mut low_stale = window("Weekly Fable 7d");
    low_stale.raw = Some("88%".to_owned());
    low_stale.fill = Some(7);
    low_stale.level = Level::Low;
    low_stale.stale = true;
    low_stale.resets = Some("3d 20h".to_owned());
    low_stale.seen = Some("40m".to_owned());
    let mut derived = window("codex pro 5h");
    derived.raw = Some("97%".to_owned());
    derived.counts = Some("48.5%".to_owned());
    derived.fill = Some(4);
    derived.why = Some(Why::Resets(1));
    derived.resets = Some("1h35m".to_owned());
    derived.seen = Some("3m".to_owned());
    let mut first = scope("claude · cc", vec![window("Session 5h"), low_stale]);
    "fable opus55x".clone_into(&mut first.profiles);
    let mut second = scope("codex · cx", vec![derived]);
    "sol61x".clone_into(&mut second.profiles);
    vec![first, second]
}

fn texts(scopes: &[Scope], width: usize, icons: bool) -> Vec<String> {
    layout(
        scopes,
        Geometry {
            width,
            rows: None,
            icons,
        },
    )
    .iter()
    .map(|line| line.text.clone())
    .collect()
}

#[test]
fn full_width_draws_header_heads_bar_used_counts_note_resets_seen() {
    assert_eq!(
        texts(&trio(), 116, true),
        [
            "  WINDOW                    USED  COUNTS   NOTE             RESETS  SEEN",
            "claude · cc   fable opus55x",
            "  Session 5h      ██░░░░░░   31%                             2h10m    2m",
            "  Weekly Fable 7d ███████░   88%           low · stale      3d 20h   40m",
            "codex · cx   sol61x",
            "  codex pro 5h    ████░░░░   97%  → 48.5%  1 reset in hand   1h35m    3m",
        ]
    );
}

#[test]
fn tones_follow_level_stale_and_position_with_words_everywhere() {
    let lines = layout(
        &trio(),
        Geometry {
            width: 116,
            rows: None,
            icons: true,
        },
    );
    let tones: Vec<Tone> = lines.iter().map(|line| line.tone.clone()).collect();
    assert_eq!(
        tones,
        [
            Tone::Dim,
            Tone::Head,
            Tone::Plain,
            Tone::Low,
            Tone::Head,
            Tone::Plain
        ]
    );
    // Level outranks stale for tone; the stale word still shows.
    assert!(lines[3].text.contains("low · stale"));
    let calm: Vec<bool> = lines.iter().map(|line| line.calm).collect();
    assert_eq!(calm, [false, false, true, false, false, true]);
}

#[test]
fn narrowing_drops_seen_then_bar_then_long_notes() {
    assert_eq!(
        texts(&trio(), 71, true),
        [
            "  WINDOW                    USED  COUNTS   NOTE             RESETS",
            "claude · cc   fable opus55x",
            "  Session 5h      ██░░░░░░   31%                             2h10m",
            "  Weekly Fable 7d ███████░   88%           low · stale      3d 20h",
            "codex · cx   sol61x",
            "  codex pro 5h    ████░░░░   97%  → 48.5%  1 reset in hand   1h35m",
        ]
    );
    assert_eq!(
        texts(&trio(), 65, true),
        [
            "  WINDOW           USED  COUNTS   NOTE             RESETS",
            "claude · cc   fable opus55x",
            "  Session 5h        31%                             2h10m",
            "  Weekly Fable 7d   88%           low · stale      3d 20h",
            "codex · cx   sol61x",
            "  codex pro 5h      97%  → 48.5%  1 reset in hand   1h35m",
        ]
    );
}

#[test]
fn short_notes_and_elided_names_hold_the_floor() {
    let lines = texts(&trio(), 50, true);
    assert_eq!(
        lines,
        [
            "  WINDOW        USED  COUNTS   NOTE         RESETS",
            "claude · cc   fable opus55x",
            "  Session 5h     31%                         2h10m",
            "  Weekly Fabl…   88%           low · stale  3d 20h",
            "codex · cx   sol61x",
            "  codex pro 5h   97%  → 48.5%  1 reset       1h35m",
        ]
    );
    assert!(lines.iter().all(|line| line.chars().count() <= 50));
}

#[test]
fn below_the_floor_table_lines_overflow_while_heads_stay_clipped() {
    let lines = texts(&trio(), 40, true);
    // Same 50-wide table as the floor frame: layout never refuses, the tmux
    // caller does by menu budget and the app clips.
    assert!(lines.iter().any(|line| line.chars().count() == 50));
    assert!(
        lines
            .iter()
            .filter(|line| !line.starts_with("  "))
            .all(|line| line.chars().count() <= 40)
    );
    let long = scope(&"gh".repeat(30), vec![window("w")]);
    let head = &texts(&[long], 40, true)[1];
    assert_eq!(head.chars().count(), 40);
    assert!(head.ends_with('…') && head.starts_with("ghgh"), "{head}");
}

#[test]
fn icons_off_draws_the_ascii_bar_in_the_same_cells() {
    let lines = texts(&trio(), 116, false);
    assert!(lines[2].contains("##......"), "{}", lines[2]);
    assert!(lines[3].contains("#######."), "{}", lines[3]);
    assert!(lines[5].contains("####...."), "{}", lines[5]);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains('█') || line.contains('░'))
    );
}

fn height_scopes() -> Vec<Scope> {
    let calm = |name: &str| {
        let mut kept = window(name);
        kept.raw = Some("10%".to_owned());
        kept.fill = Some(1);
        kept.resets = Some("1h".to_owned());
        kept.seen = Some("1m".to_owned());
        kept
    };
    let mut low = calm("b2");
    low.raw = Some("85%".to_owned());
    low.fill = Some(7);
    low.level = Level::Low;
    let mut unreadable = scope("c", Vec::new());
    unreadable.only = Some("this client reports no quota".to_owned());
    vec![
        scope("a", vec![calm("a1"), calm("a2")]),
        scope("b", vec![calm("b1"), low]),
        unreadable,
    ]
}

fn height_texts(scopes: &[Scope], rows: usize) -> Vec<String> {
    layout(
        scopes,
        Geometry {
            width: 116,
            rows: Some(rows),
            icons: true,
        },
    )
    .iter()
    .map(|line| line.text.clone())
    .collect()
}

#[test]
fn short_height_drops_calm_windows_bottom_up_and_admits_both_counts() {
    assert_eq!(
        height_texts(&height_scopes(), 8),
        [
            "  WINDOW           USED  NOTE  RESETS  SEEN",
            "a",
            "  a1     █░░░░░░░   10%            1h    1m",
            "b",
            "  b2     ███████░   85%  low       1h    1m",
            "c",
            "  this client reports no quota",
            "+2 calm windows — ae quota",
        ]
    );
    assert_eq!(
        height_texts(&height_scopes(), 6)[5],
        "+1 calm scope, 1 calm window — ae quota"
    );
    assert_eq!(
        height_texts(&[scope("a", vec![window("a1")])], 2)[1],
        "+1 calm scope — ae quota"
    );
    let overflow = height_texts(&height_scopes(), 2);
    assert!(overflow.len() > 2, "trouble returns whole: {overflow:?}");
    assert!(overflow.iter().any(|line| line.contains("85%")));
    assert!(overflow.iter().any(|line| line.contains("no quota")));
    assert!(
        overflow
            .last()
            .is_some_and(|line| line.contains("— ae quota"))
    );
}

#[test]
fn no_reading_wraps_and_unavailable_is_one_head() {
    let mut old = scope("claude · cc-old", Vec::new());
    old.profiles = "opus5x".to_owned();
    old.only = Some(
        "no reading ae can trust (last report 2d 4h ago) — run /usage in a claude session"
            .to_owned(),
    );
    assert_eq!(
        texts(&[old], 76, true),
        [
            "  WINDOW           USED  NOTE  RESETS  SEEN",
            "claude · cc-old   opus5x",
            "  no reading ae can trust (last report 2d 4h ago) — run /usage in a claude",
            "  session",
        ]
    );
    assert_eq!(
        unavailable(),
        vec![Scope {
            title: "quota: unavailable".to_owned(),
            profiles: String::new(),
            account: None,
            only: None,
            windows: Vec::new(),
        }]
    );
    assert_eq!(
        texts(&unavailable(), 76, true),
        [
            "  WINDOW           USED  NOTE  RESETS  SEEN",
            "quota: unavailable",
        ]
    );
}

/// quotacache: a kept-prior note draws stale above surviving windows — the
/// contract the cached overlay relies on (kept rows plus a failure note,
/// never merged).
#[test]
fn kept_prior_note_draws_stale_above_surviving_windows() {
    let mut kept = scope("agy · ag", vec![window("gemini 5h")]);
    kept.only = Some("last refresh failed — agy /quota gave no usable answer".to_owned());
    let lines = layout(
        &[kept],
        Geometry {
            width: 76,
            rows: None,
            icons: true,
        },
    );
    let note = lines
        .iter()
        .position(|line| line.text.contains("last refresh failed"))
        .expect("the note");
    assert_eq!(lines[note].tone, Tone::Stale);
    let first_window = lines
        .iter()
        .position(|line| line.text.contains("gemini 5h"))
        .expect("the kept window");
    assert!(note < first_window, "note above windows");
}
