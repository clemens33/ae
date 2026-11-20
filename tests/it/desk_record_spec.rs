//! Acceptance for the owner board's input and persisted record.

#![allow(
    clippy::expect_used,
    reason = "failed acceptance fixtures need a label"
)]

use ae::desk::{encode, parse, parse_input};

const GOOD: &str = concat!(
    "ae-desk 1\nupdated 2026-11-20T08:40:12Z\n",
    "D2\tChoose & verify <now>\thttps://example.test/a?q=1&n=2\t/tmp/board notes/α.md\n",
    "D17\tPublish later\n",
    "C9\tStage the release\n",
    "R3\tRead the gate\n",
    "W7\tWait for the owner\n",
    "L1\tRead the release notes\thttps://example.test/releases\n",
);

#[test]
fn the_stored_board_preserves_all_sections_ids_and_link_destinations() {
    let parsed = parse(GOOD.as_bytes()).expect("canonical board");
    assert_eq!(encode(&parsed), GOOD.as_bytes());
    let labels_only = b"ae-desk 1\nupdated 2026-11-20T08:40:12Z\n";
    assert_eq!(
        encode(&parse(labels_only).expect("empty board")),
        labels_only
    );

    let inputs = [
        "DECIDE\nD17 | Publish later\nD2 | Choose & verify <now> | https://example.test/a?q=1&n=2 | /tmp/board notes/α.md\nCOMING\nC9 | Stage the release\nRUNNING\nR3 | Read the gate\nWAITING ON OTHERS\nW7 | Wait for the owner\nLINKS\nL1 | Read the release notes | https://example.test/releases\n",
        "WAITING ON OTHERS\nW7 | Wait for a report\n",
        "DECIDE\nCOMING\nRUNNING\nWAITING ON OTHERS\nLINKS\n",
        "DECIDE\nD99 | Verify | https://a.test/x | https://b.test/y | /tmp/spec/report.md\n",
    ];
    for input in inputs {
        assert!(parse_input(input.as_bytes()).is_ok(), "refused {input:?}");
    }
}

#[test]
fn malformed_records_do_not_become_partial_boards() {
    let cases = [
        ("other version", GOOD.replacen("ae-desk 1", "ae-desk 2", 1)),
        (
            "no time",
            GOOD.replacen("updated 2026-11-20T08:40:12Z\n", "", 1),
        ),
        (
            "malformed time",
            GOOD.replacen("2026-11-20T08:40:12Z", "yesterday", 1),
        ),
        (
            "duplicate time",
            GOOD.replacen("D2\t", "updated 2026-11-20T08:40:12Z\nD2\t", 1),
        ),
        ("duplicate id", GOOD.replacen("D17\t", "D2\t", 1)),
        ("out of order", GOOD.replacen("D17\t", "D1\t", 1)),
        ("unknown section id", GOOD.replacen("W7\t", "H7\t", 1)),
        (
            "link without a host",
            GOOD.replacen("https://example.test/releases", "https://", 1),
        ),
        (
            "relative path",
            GOOD.replacen("/tmp/board notes/α.md", "tmp/board notes/α.md", 1),
        ),
        (
            "control in destination",
            GOOD.replacen("/tmp/board notes/α.md", "/tmp/a\x1b]8;;evil", 1),
        ),
        ("torn final line", GOOD.trim_end_matches('\n').to_owned()),
    ];
    for (case, data) in cases {
        assert!(parse(data.as_bytes()).is_err(), "accepted {case}");
    }
    let mut invalid_utf8 = GOOD.as_bytes().to_vec();
    invalid_utf8.push(0xff);
    assert!(parse(&invalid_utf8).is_err());
}

#[test]
fn bad_input_refuses_the_whole_update_and_reports_the_offending_line() {
    let cases: [(&str, usize); 13] = [
        ("DECIDE\nUNKNOWN\n", 2),
        ("DECIDE\nDECIDE\n", 2),
        ("LINKS\nRUNNING\n", 2),
        ("DECIDE\nR1 | wrong section\n", 2),
        ("DECIDE\nD1 | first\nD1 | duplicate\n", 3),
        ("DECIDE\nD01 | leading zero\n", 2),
        ("DECIDE\nD0 | below range\n", 2),
        ("DECIDE\nD100 | above range\n", 2),
        ("DECIDE\nD1 | literal\x1b text\n", 2),
        ("DECIDE\nD1 | go | ./relative\n", 2),
        ("DECIDE\nD1 | go | /tmp/../report.md\n", 2),
        ("DECIDE\nD1 | go | https://\n", 2),
        ("DECIDE\nupdated 2026-11-20T08:40:12Z\n", 2),
    ];
    for (data, line) in cases {
        let why = parse_input(data.as_bytes()).expect_err("bad input must fail");
        assert_eq!(why.line, line, "{data:?}");
    }
    for data in [
        String::new(),
        "   \n".to_owned(),
        "DECIDE\nD1 | \n".to_owned(),
        "DECIDE\nD1 | a\tb\n".to_owned(),
        "DECIDE\nD1 | one | https://a.test/ | https://b.test/ | https://c.test/ | https://d.test/\n".to_owned(),
        format!("DECIDE\nD1 | {}\n", "α".repeat(161)),
        format!("DECIDE\nD1 | go | https://a.test/{}\n", "x".repeat(300)),
        format!("DECIDE\n{}", " ".repeat(4090)),
    ] {
        assert!(parse_input(data.as_bytes()).is_err(), "accepted {data:?}");
    }
    let items = (1..=17).fold("DECIDE\n".to_owned(), |mut body, id| {
        use std::fmt::Write as _;
        writeln!(body, "D{id} | item").expect("append to String");
        body
    });
    assert!(parse_input(items.as_bytes()).is_err(), "accepted item 17");
    assert!(parse_input(b"DECIDE\nD1 | \xff\n").is_err());
    let max_text = format!("DECIDE\nD1 | {}\n", "α".repeat(160));
    assert!(parse_input(max_text.as_bytes()).is_ok());
}

#[test]
fn accepted_input_at_the_byte_ceiling_survives_the_stored_header() {
    use ae::desk::Desk;
    use ae::time::Timestamp;
    use std::fmt::Write as _;

    let mut input = String::from("DECIDE\n");
    for id in 1..=6 {
        writeln!(input, "D{id} | {}", "🦦".repeat(160)).expect("fill input");
    }
    let closing_text_bytes = 4096 - input.len() - "D7 | \n".len();
    let closing_text = "🦦".repeat(closing_text_bytes / 4) + &"x".repeat(closing_text_bytes % 4);
    writeln!(input, "D7 | {closing_text}").expect("fill boundary");
    assert_eq!(input.len(), 4096);
    let items = parse_input(input.as_bytes()).expect("input cap is inclusive");
    let original = Desk {
        updated: Timestamp::from_epoch(1_795_164_012),
        items,
    };
    let bytes = encode(&original);
    assert!(
        bytes.len() > 4096,
        "the fixture exceeds the input cap after encoding"
    );
    assert_eq!(parse(&bytes).expect("stored bytes read back"), original);
    input.push('\n');
    assert!(
        parse_input(input.as_bytes()).is_err(),
        "one allowed LF still exceeds the cap"
    );
}

#[test]
fn delta_uses_stable_ids_and_notices_a_link_only_change() {
    use ae::desk::{Id, Section, diff};

    let old = parse_input(
        b"DECIDE\nD8 | removed\nD9 | Same words | https://old.test/\nRUNNING\nR2 | still running\n",
    )
    .expect("old record");
    let new = parse_input(
        b"DECIDE\nD9 | Same words | https://new.test/\nD11 | added\nRUNNING\nR2 | still running\n",
    )
    .expect("new record");
    let delta = diff(&old, &new);
    let decide = |number| Id {
        section: Section::Decide,
        number,
    };
    assert_eq!(delta.removed, [decide(8)]);
    assert_eq!(delta.changed, [decide(9)]);
    assert_eq!(delta.added, [decide(11)]);
    assert!(diff(&new, &new).is_empty());
}
