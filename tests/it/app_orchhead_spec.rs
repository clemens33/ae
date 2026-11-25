//! Frozen acceptance for orchhead, the parts outside the app's frame: a saved
//! view that still carries the retired board preference (`|desk=…`) reads and
//! keeps every other setting, writes the line without it, and a malformed
//! spelling stays refused; `docs/app.md` says what the orchestrator header
//! shows and that the board column is gone. None of it names a field or an API
//! the slice removes, so it compiles before and after.
#![allow(
    clippy::expect_used,
    reason = "acceptance parses fixed literals and fails with the one that broke"
)]

use ae::app::handover::{self, Handover, OrchestratorView, PanelMode, View};

const ORCH_ID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const V1: &str = "v1|%7|2026.10.99|1760259000|web|6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11|agents|colead|31|9|2|0|1|4";
const APP_DOC: &str = include_str!("../../docs/app.md");

/// The three canonical spellings of one view: closed, panel open, zoomed main
/// column with the panel open.
fn plain_views() -> Vec<Handover> {
    let closed = handover::parse(V1).expect("the canonical literal");
    let mut open = closed.clone();
    open.view.orchestrator = Some(OrchestratorView {
        id: ORCH_ID.into(),
        mode: PanelMode::Full,
        pages: 3,
        rows: 7,
    });
    let mut zoomed = open.clone();
    zoomed.view.zoom = true;
    vec![closed, open, zoomed]
}

fn plain_lines() -> Vec<String> {
    plain_views().iter().map(Handover::render).collect()
}

/// Every field a restart must keep, whichever way the board preference reads.
fn kept(held: &Handover) -> impl PartialEq + std::fmt::Debug {
    let view: &View = &held.view;
    // A tuple past twelve fields has no `PartialEq` or `Debug`: three groups.
    (
        (
            held.pane.clone(),
            held.version.clone(),
            held.at,
            view.session.clone(),
            view.id.clone(),
            view.focus.clone(),
        ),
        (
            view.sidebar,
            view.list,
            view.pages,
            view.rows,
            view.list_top,
            view.body_top,
        ),
        (view.panel, view.zoom, view.orchestrator.clone()),
    )
}

const LEGACY: [&str; 7] = [
    "hidden",
    "44",
    "44,hidden",
    "tab=links",
    "tab=running",
    "hidden,tab=coming",
    "44,hidden,tab=waiting",
];

#[test]
fn a_live_line_with_a_retired_board_suffix_keeps_every_setting_and_writes_it_no_more() {
    for plain in plain_lines() {
        let want = handover::parse(&plain).expect("the plain line");
        for suffix in LEGACY {
            let line = format!("{plain}|desk={suffix}");
            let read = handover::parse(&line).unwrap_or_else(|| panic!("refused: {line}"));
            assert_eq!(kept(&read), kept(&want), "{line}");
            assert_eq!(
                read.render(),
                plain,
                "the suffix is not written back: {line}"
            );
        }
    }
}

#[test]
fn a_durable_file_with_a_retired_board_suffix_reads_and_is_rewritten_without_it() {
    for held in plain_views() {
        let durable = Handover::durable("2026.11.113", 1_760_259_000, held.view);
        let plain = handover::render_file(&durable);
        let line = plain.trim_end_matches('\n');
        let want = handover::parse_file(plain.as_bytes()).expect("the plain file");
        for suffix in LEGACY {
            let text = format!("{line}|desk={suffix}\n");
            let read = handover::parse_file(text.as_bytes())
                .unwrap_or_else(|| panic!("refused: {text:?}"));
            assert_eq!(kept(&read), kept(&want), "{text:?}");
            assert_eq!(handover::render_file(&read), plain, "{text:?}");
        }
    }
}

#[test]
fn a_malformed_retired_suffix_stays_refused() {
    let lines = plain_lines();
    let plain = &lines[0];
    for bad in [
        "",
        ",hidden",
        "hidden,hidden",
        "hidden,44",
        "44,hidden,hidden",
        "44,shown",
        "44, hidden",
        "Hidden",
        "tab=decide",
        "tab=",
        "tab=unknown",
        "hidden|desk=44",
    ] {
        let line = format!("{plain}|desk={bad}");
        assert_eq!(handover::parse(&line), None, "{line}");
    }
    assert_eq!(handover::parse(&format!("{plain}||desk=hidden")), None);
    assert_eq!(handover::parse(&format!("{plain}|desk=44|desk=45")), None);
}

#[test]
fn the_app_doc_says_what_the_orchestrator_header_shows_and_that_the_board_column_is_gone() {
    for gone in ["`b hide`", "`b show`", "`B board`", "`B chat`", "`B/Esc`"] {
        assert!(!APP_DOC.contains(gone), "docs/app.md still says {gone:?}");
    }
    let key_row = |row: &&str| row.starts_with("| `b`") || row.starts_with("| `B`");
    assert_eq!(
        APP_DOC.lines().filter(key_row).count(),
        0,
        "a key table row still serves the board"
    );
    for kept in [
        "fleet ● 7 working · ⚠ 2 wait you · ≡ 4 sessions",
        "the sidebar's Working",
        "no board column",
        "desk check",
    ] {
        assert!(APP_DOC.contains(kept), "docs/app.md lacks {kept:?}");
    }
}
