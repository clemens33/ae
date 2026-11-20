//! Acceptance for visible board rows and complete link destinations.

#![allow(
    clippy::expect_used,
    reason = "failed acceptance fixtures need a label"
)]

use ae::console::deskview::{RowKind, fit, rows};
use ae::desk::parse;

const WEB: &str = "https://example.test/a-long-board-link?x=1&y=2";
const FILE_URL: &str = "file:///tmp/board%20notes/%CE%B1.md";
const BOARD: &str = concat!(
    "ae-desk 1\nupdated 2026-11-20T08:40:12Z\n",
    "D7\tReview the blue option\thttps://example.test/a-long-board-link?x=1&y=2\t/tmp/board notes/α.md\n",
    "C1\tPrepare the release\n",
    "R2\tWrite the draft\n",
    "W3\tWait for the reply\n",
    "L4\tArchive the receipts\n",
);

#[test]
fn the_board_shows_its_update_time_and_every_wrapped_link_targets_the_whole_destination() {
    let desk = parse(BOARD.as_bytes()).expect("board display fixture");
    let wide = rows(&desk, u16::MAX, Some("+0200"));
    assert_eq!(wide[0].kind, RowKind::Header);
    assert!(wide[0].text.contains("10:40:12"), "{:?}", wide[0]);
    let headings: Vec<_> = wide
        .iter()
        .filter(|row| row.kind == RowKind::Heading)
        .map(|row| row.text.trim())
        .collect();
    assert_eq!(
        headings,
        ["DECIDE", "COMING", "RUNNING", "WAITING ON OTHERS", "LINKS"]
    );
    assert!(
        wide.iter()
            .any(|row| row.text.contains("D7") && row.text.contains("Review the blue option"))
    );
    assert!(
        wide.iter()
            .any(|row| row.text.contains("/tmp/board notes/α.md"))
    );

    let narrow = rows(&desk, 20, Some("bad offset"));
    assert!(narrow.iter().all(|row| row.text.chars().count() <= 20));
    let links: Vec<_> = narrow
        .iter()
        .flat_map(|row| {
            row.links.iter().map(move |link| {
                let visible = row
                    .text
                    .get(link.range.clone())
                    .expect("valid link byte range");
                assert!(!visible.trim().is_empty(), "visible link span: {row:?}");
                link.url.as_str()
            })
        })
        .collect();
    assert!(
        links.iter().all(|url| matches!(*url, WEB | FILE_URL)),
        "{links:?}"
    );
    assert!(
        links.iter().filter(|url| **url == WEB).count() >= 2,
        "the web link wraps"
    );
    assert!(
        links.iter().filter(|url| **url == FILE_URL).count() >= 2,
        "the file path wraps"
    );
    assert!(
        rows(&desk, u16::MAX, Some("bad offset"))[0]
            .text
            .contains("08:40:12")
    );
}

#[test]
fn a_short_column_keeps_its_height_and_marks_the_omission() {
    let desk = parse(BOARD.as_bytes()).expect("board display fixture");
    let wide = rows(&desk, 35, None);
    assert!(wide.len() > 5);
    assert_eq!(fit(wide.clone(), u16::MAX), wide);
    assert!(fit(wide.clone(), 0).is_empty());
    let short = fit(wide, 5);
    assert!(short.len() <= 5 && !short.is_empty());
    let end = short.last().expect("omission row");
    assert_eq!(end.kind, RowKind::More);
    assert!(end.text.contains("/board") && end.text.contains("more"));
    assert!(end.links.is_empty());
}
