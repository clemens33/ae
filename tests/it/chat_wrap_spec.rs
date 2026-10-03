//! Independent acceptance pins for chat-wrap, A1, and R-A2/R-A3.

#![allow(
    clippy::expect_used,
    reason = "acceptance owns isolated journal and terminal fixtures"
)]

use ae::console::input::{Command, command, outcome_line};
use ae::console::lane::{Item, Kind, Lane};
use ae::console::submit::{Outcome, close_owned, submit};
use ae::console::view::{Printed, Style};
use ae::events::Event;
use ae::store;
use ae::theme::Look;
use std::cell::Cell;

const TIME: i64 = 1_790_748_060_000_000;
const ID: &str = "ae-20261003T070000Z-12345678";

/// Accept only ae's SGR escapes; source text may never smuggle a terminal command.
fn unstyled(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        assert_eq!(chars.next(), Some('['));
        loop {
            let next = chars.next().expect("terminated SGR");
            if next == 'm' {
                break;
            }
            assert!(next.is_ascii_digit() || next == ';', "SGR only");
        }
    }
    out
}

/// Requirement oracle, independent of `event_text`'s implementation. The bar is
/// ae's known one-cell glyph; text uses the conservative rule in R-A2/R-A3.
fn row_cells(row: &str) -> usize {
    let mut chars = row.chars();
    assert!(matches!(chars.next(), Some('▌' | '|')), "{row:?}");
    chars.fold(1, |column, ch| {
        column
            + match ch {
                '\t' => 8 - column % 8,
                ' '..='~' => 1,
                _ => 2,
            }
    })
}

fn letters(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn lane(kind: Kind, body: &str) -> Lane {
    Lane {
        items: vec![Item {
            micros: TIME,
            kind,
            body: body.to_owned(),
            record: None,
        }],
        coverage: vec![],
    }
}

fn styled(icons: bool) -> Style {
    Style::resolve(
        true,
        Some(Look::read(
            if icons { "on" } else { "off" },
            "darcula",
            "",
            "",
        )),
        Some("+0000"),
        "lead",
    )
}

fn entry_rows(text: &str) -> Vec<String> {
    unstyled(text)
        .lines()
        .filter(|row| !row.is_empty() && !row.starts_with("# "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn chat_wrap_every_visual_header_body_and_status_row_keeps_voice_bar_and_fits() {
    let to = "lead with a very long descriptive seat name";
    let body = "Long prose words wrap across several rows.\nhttps://example.invalid/abcdefghijklmnopqrstuvwxyz0123456789";
    let status = "sent; the draft is kept because later submission needs human review";
    let data = lane(
        Kind::Asked {
            to: to.to_owned(),
            id: ID.to_owned(),
            uncertain: false,
        },
        body,
    );
    for icons in [false, true] {
        let glyph = if icons { '▌' } else { '|' };
        for width in [3, 4, 5, 7, 12, 20, 40, 80] {
            let mut printed = Printed::styled(styled(icons));
            printed.set_width(Some(width));
            printed.outcome(ID, status.to_owned());
            let rendered = printed.step(&data, 0, true);
            let rows = entry_rows(&rendered);
            assert!(rows.len() > 3, "long entry wraps: {width}: {rows:?}");
            for row in &rows {
                assert!(row.starts_with(glyph), "{row:?}");
                assert!(row_cells(row) <= width, "width {width}: {row:?}");
            }
            let recovered: String = rows
                .iter()
                .flat_map(|row| row.chars().skip(1))
                .filter(|ch| !ch.is_whitespace())
                .collect();
            assert_eq!(
                recovered,
                letters(&format!("06:01:00 you → {to}{body}{status}"))
            );
            assert!(!rendered.contains(ID));
            // Human entry bars keep Darcula's human/title hue, independent of
            // the status tone and header timestamp. Each rendered row gets it.
            let bar = format!("\x1b[38;2;{}m{glyph}\x1b[0m", human_rgb());
            let barred = rendered.lines().filter(|row| row.starts_with(&bar)).count();
            assert_eq!(barred, rows.len(), "bar hue on every visual row");
        }
    }
}

fn human_rgb() -> String {
    let hex = Look::read("on", "darcula", "", "").palette.title;
    [1, 3, 5]
        .map(|at| {
            u8::from_str_radix(&hex[at..at + 2], 16)
                .expect("palette hex")
                .to_string()
        })
        .join(";")
}

#[test]
fn chat_wrap_wide_scalars_tabs_and_continuation_indent_stay_inside_width() {
    for body in [
        "中中中中中",
        "a中bé中c",
        "\t中中\tend",
        "  indented words and 中中中 text",
    ] {
        for width in [3, 4, 5, 6, 9, 12, 20] {
            let data = lane(
                Kind::Said {
                    who: "lead".to_owned(),
                },
                body,
            );
            let mut printed = Printed::styled(styled(true));
            printed.set_width(Some(width));
            let text = printed.step(&data, 0, true);
            let rows = entry_rows(&text);
            for row in &rows {
                assert!(!row.contains('\t'), "dressed tabs expand: {row:?}");
                assert!(row_cells(row) <= width, "width {width}: {row:?}");
            }
            let recovered: String = rows.iter().flat_map(|row| row.chars().skip(1)).collect();
            assert_eq!(
                letters(&recovered),
                letters(&format!("06:01:00 said lead{body}"))
            );
        }
    }
    let data = lane(
        Kind::Said {
            who: "lead".to_owned(),
        },
        "  alpha beta gamma delta epsilon zeta",
    );
    let mut printed = Printed::styled(styled(true));
    printed.set_width(Some(20));
    let rows = entry_rows(&printed.step(&data, 0, true));
    assert!(rows.len() >= 3);
    for row in rows.iter().skip(1) {
        assert!(
            row.starts_with("▌    "),
            "logical indent on all body rows: {row:?}"
        );
    }
}

#[test]
fn chat_wrap_plain_bytes_keep_tabs_and_do_not_wrap_at_any_known_width() {
    let body = "\t中 abcdefghijklmnopqrstuvwxyz\n  another body line";
    let expected = "# 2026-09-30 UTC\n## 06:01:00 said lead\n  \t中 abcdefghijklmnopqrstuvwxyz\n    another body line\n\n";
    for width in [Some(0), Some(3), Some(4), Some(20), None] {
        for style in [
            Style::PLAIN,
            Style::resolve(
                true,
                Some(Look::read("on", "darcula", "off", "")),
                None,
                "lead",
            ),
        ] {
            let mut printed = Printed::styled(style);
            printed.set_width(width);
            assert_eq!(
                printed.step(
                    &lane(
                        Kind::Said {
                            who: "lead".to_owned()
                        },
                        body
                    ),
                    0,
                    true
                ),
                expected
            );
        }
    }
}

#[test]
fn chat_wrap_unobservable_width_keeps_the_dressed_unwrapped_residual() {
    let body = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz 中\tend";
    let mut printed = Printed::styled(styled(true));
    printed.set_width(None);
    let text = printed.step(
        &lane(
            Kind::Said {
                who: "lead".to_owned(),
            },
            body,
        ),
        0,
        true,
    );
    assert!(text.contains('\x1b'), "unknown width keeps look");
    assert_eq!(
        unstyled(&text),
        format!("# 2026-09-30 +0000\n▌ 06:01:00 said lead\n▌  {body}\n\n")
    );
}

#[test]
fn chat_wrap_resize_changes_new_entry_width_without_reprinting_old_entries() {
    let old = Item {
        micros: TIME,
        kind: Kind::Said {
            who: "lead".to_owned(),
        },
        body: "old entry words".to_owned(),
        record: None,
    };
    let mut data = Lane {
        items: vec![old],
        coverage: vec![],
    };
    let mut printed = Printed::styled(styled(true));
    printed.set_width(Some(20));
    assert!(unstyled(&printed.step(&data, 0, true)).contains("old entry words"));
    printed.set_width(Some(4));
    assert_eq!(
        printed.step(&data, 0, true),
        "",
        "resize does not replay old rows"
    );
    data.items.push(Item {
        micros: TIME,
        kind: Kind::Said {
            who: "lead".to_owned(),
        },
        body: "new entry 中中".to_owned(),
        record: None,
    });
    let text = printed.step(&data, 0, true);
    let rows = entry_rows(&text);
    assert!(
        rows.iter().all(|row| row_cells(row) <= 4),
        "new width applies: {rows:?}"
    );
    let recovered: String = rows.iter().flat_map(|row| row.chars().skip(1)).collect();
    assert_eq!(
        letters(&recovered),
        letters("06:01:00 said leadnew entry 中中")
    );
}

#[test]
fn chat_wrap_ask_answer_closed_and_unadmitted_headers_hide_request_ids_only() {
    let cases = [
        (
            Kind::Asked {
                to: "lead".to_owned(),
                id: ID.to_owned(),
                uncertain: false,
            },
            "you → lead",
        ),
        (
            Kind::Asked {
                to: "lead".to_owned(),
                id: ID.to_owned(),
                uncertain: true,
            },
            "you → lead · uncertain: check the lead pane",
        ),
        (
            Kind::NotDelivered {
                to: "lead".to_owned(),
                id: ID.to_owned(),
            },
            "you → lead · not delivered",
        ),
        (Kind::Closed { id: ID.to_owned() }, "you closed an ask"),
        (
            Kind::Answer {
                seat: "lead".to_owned(),
                id: ID.to_owned(),
                follow_up: 2,
                late: true,
                gap: None,
                speaker: Some("new-profile".to_owned()),
            },
            "lead answers · follow-up 2 · late (closed) · speaker lead now new-profile",
        ),
        (
            Kind::Unadmitted {
                from: "lead".to_owned(),
                id: ID.to_owned(),
                why: "wrong source".to_owned(),
            },
            "lead reply · not admitted: wrong source · preview (600-char summary)",
        ),
    ];
    for (kind, expected_tag) in cases {
        let mut plain = Printed::default();
        let expected = format!("# 2026-09-30 UTC\n## 06:01:00 {expected_tag}\n  retained body\n\n");
        assert_eq!(
            plain.step(&lane(kind.clone(), "retained body"), 0, true),
            expected
        );
        let mut dressed = Printed::styled(styled(true));
        dressed.set_width(Some(20));
        let text = dressed.step(&lane(kind, "retained body"), 0, true);
        assert!(!text.contains("ae-2026"), "{text}");
    }
}

#[test]
fn chat_wrap_bare_close_is_optional_id_while_explicit_close_stays_supported() {
    let pair = vec!["lead".to_owned(), "colead".to_owned()];
    assert!(matches!(command(b"/close", &pair), Command::Close(None)));
    assert!(matches!(command(b"/close   ", &pair), Command::Close(None)));
    assert!(
        matches!(command(format!("/close {ID}").as_bytes(), &pair), Command::Close(Some(id)) if id == ID)
    );
    assert!(matches!(command(b"/close x y", &pair), Command::Refused(_)));
}

fn receipt(actor: &str, action: &str, id: &str) -> String {
    let (target, routing, stamp) = if action == "reply" {
        ("console:local", "actor", "caller")
    } else {
        ("lead", "target", "target")
    };
    format!(
        "{{\"ts\":\"2026-10-03T07:00:00Z\",\"actor\":\"{actor}\",\"action\":\"{action}\",\"target\":\"{target}\",\"ref\":\"{id}\",\"{routing}_slot\":\"main\",\"{routing}_session\":\"s\",\"{stamp}_server\":\"/private/test.sock\",\"{stamp}_pane\":\"%1\",\"{stamp}_session_uuid\":\"0199c0de-bbbb-4890-abcd-ef0123456789\",\"summary\":\"retained request words\"}}\n"
    )
}

fn journal_events(bytes: &[u8]) -> Vec<Event> {
    std::str::from_utf8(bytes)
        .expect("fixture journal UTF-8")
        .lines()
        .map(|line| Event::parse_line(line).expect("fixture event"))
        .collect()
}

#[test]
fn chat_wrap_bare_close_withdraws_newest_own_open_ask_by_journal_order() {
    let scratch = super::cli::OwnedScratch::root("chatwrap", "newest");
    let saved = store::open(scratch.path());
    let older = "ae-20261003T070000Z-ffffffff";
    let newer = "ae-20261003T070000Z-00000001";
    let foreign = "ae-20261003T070000Z-00000002";
    for line in [
        receipt("console:local", "ask", older),
        receipt("console:local", "ask", newer),
        receipt("lead", "ask", foreign),
    ] {
        saved.append_event(&line).expect("seed journal");
    }
    assert_eq!(close_owned(scratch.path(), "s", None, || Ok(())), Ok(()));
    let events = journal_events(&saved.container());
    let cancels: Vec<_> = events
        .iter()
        .filter(|event| event.action == "cancel")
        .collect();
    assert_eq!(cancels.len(), 1);
    assert_eq!(cancels[0].actor, "console:local");
    assert_eq!(cancels[0].reference.as_deref(), Some(newer));
    assert_eq!(ae::console::lane::open_asks(&events, "s"), vec![older]);
    assert_eq!(close_owned(scratch.path(), "s", None, || Ok(())), Ok(()));
    let bytes = saved.container();
    assert!(ae::console::lane::open_asks(&journal_events(&bytes), "s").is_empty());
    let refusal = close_owned(scratch.path(), "s", None, || Ok(())).expect_err("no own open ask");
    assert!(refusal.contains("no open ask"), "{refusal}");
    assert_eq!(saved.container(), bytes, "no-open refusal never appends");
}

#[test]
fn chat_wrap_close_skips_answered_latest_and_checks_owner_before_writing() {
    let scratch = super::cli::OwnedScratch::root("chatwrap", "answered");
    let saved = store::open(scratch.path());
    let older = "ae-20261003T070000Z-00000001";
    let answered = "ae-20261003T070000Z-00000002";
    for line in [
        receipt("console:local", "ask", older),
        receipt("console:local", "ask", answered),
        receipt("lead", "reply", answered),
    ] {
        saved.append_event(&line).expect("seed journal");
    }
    let before = saved.container();
    let called = Cell::new(false);
    let refused = close_owned(scratch.path(), "s", None, || {
        called.set(true);
        Err("input owned by window @7".to_owned())
    });
    assert!(called.get());
    assert_eq!(refused, Err("input owned by window @7".to_owned()));
    assert_eq!(saved.container(), before);
    assert_eq!(close_owned(scratch.path(), "s", None, || Ok(())), Ok(()));
    let events = journal_events(&saved.container());
    let canceled = events
        .iter()
        .find(|event| event.action == "cancel")
        .expect("cancel event");
    assert_eq!(
        canceled.reference.as_deref(),
        Some(older),
        "bare close chooses the newest unanswered ask"
    );
}

#[test]
fn chat_wrap_explicit_close_can_still_withdraw_an_older_own_ask() {
    let scratch = super::cli::OwnedScratch::root("chatwrap", "explicit");
    let saved = store::open(scratch.path());
    let newer = "ae-20261003T070000Z-00000002";
    for id in [ID, newer] {
        saved
            .append_event(&receipt("console:local", "ask", id))
            .expect("seed ask");
    }
    assert_eq!(
        close_owned(scratch.path(), "s", Some(ID), || Ok(())),
        Ok(())
    );
    let events = journal_events(&saved.container());
    assert_eq!(ae::console::lane::open_asks(&events, "s"), vec![newer]);
}

#[test]
fn chat_wrap_outcomes_and_open_cap_refusal_keep_words_without_request_ids() {
    let cases = [
        (Outcome::Sent(ID.to_owned(), None), "sent"),
        (
            Outcome::Sent(ID.to_owned(), Some("the draft is kept (locked)".to_owned())),
            "sent; the draft is kept (locked)",
        ),
        (
            Outcome::Uncertain(ID.to_owned()),
            "uncertain: check lead pane (prefix H)",
        ),
        (Outcome::NotDelivered(ID.to_owned()), "not delivered"),
        (
            Outcome::Unknown(ID.to_owned(), "read failed".to_owned()),
            "no record of it; it may have been delivered - check lead pane (prefix H)",
        ),
    ];
    for (outcome, expected) in cases {
        assert_eq!(outcome_line(&outcome, "lead"), expected);
    }
    let scratch = super::cli::OwnedScratch::root("chatwrap", "cap");
    let saved = store::open(scratch.path());
    for index in 0..5 {
        let id = format!("ae-20261003T070000Z-{index:08x}");
        saved
            .append_event(&receipt("console:local", "ask", &id))
            .expect("seed open ask");
    }
    let before = saved.container();
    let delivered = Cell::new(false);
    let refused = submit(
        scratch.path(),
        "s",
        b"sixth",
        ID,
        || Ok(()),
        || {
            delivered.set(true);
            String::new()
        },
    )
    .expect_err("five-open cap");
    assert!(!delivered.get());
    assert!(refused.contains("5 requests open"), "{refused}");
    assert!(!refused.contains("ae-2026"), "{refused}");
    assert_eq!(saved.container(), before, "cap never delivers or appends");
}
