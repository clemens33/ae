//! Pins the bounded mutation run found loose beside the chat-wrap spec: a
//! header whose timestamp itself splits across rows.

use ae::console::lane::{Item, Kind, Lane};
use ae::console::view::{Printed, Style};
use ae::theme::Look;

fn sgr(hex: &str) -> String {
    let at = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    format!("\x1b[38;2;{};{};{}m", at(1), at(3), at(5))
}

#[test]
fn a_timestamp_split_across_rows_ends_dim_and_the_words_after_it_wear_the_speakers_hue() {
    let look = Look::read("on", "darcula", "", "");
    let style = Style::resolve(true, Some(look), Some("+0000"), "lead");
    let lane = Lane {
        items: vec![Item {
            micros: 1_790_748_060_000_000,
            kind: Kind::Said {
                who: "lead".to_owned(),
            },
            body: "x".to_owned(),
            record: None,
        }],
        coverage: vec![],
    };
    let mut printed = Printed::styled(style);
    printed.set_width(Some(9));
    let text = printed.step(&lane, 0, true);
    let (dim, voice) = (sgr(look.palette.dim), sgr(look.palette.working));
    // Width 9 breaks "06:01:00 said lead" after seven cells, so the second row
    // holds the stamp's last cell and the first word.
    assert!(text.contains(&format!("{dim}06:01:0\x1b[0m\n")), "{text:?}");
    assert!(
        text.contains(&format!("{dim}0\x1b[0m {voice}said\x1b[0m\n")),
        "{text:?}"
    );
}

/// The indented body rows a decision card with `body` prints.
fn card_rows(body: &str) -> Vec<String> {
    let lane = Lane {
        items: vec![Item {
            micros: 1_790_748_060_000_000,
            kind: Kind::Card {
                seat: "lead".to_owned(),
            },
            body: body.to_owned(),
            record: None,
        }],
        coverage: vec![],
    };
    let shown = Printed::default().step(&lane, 0, true);
    let rows = shown.lines().filter(|line| line.starts_with("  "));
    rows.map(str::to_owned).collect()
}

#[test]
fn an_option_label_at_column_zero_of_a_decision_card_stays_one_whole_line() {
    // A reason may open on its first option: `1 keep: fast | 2 wait`. A label at
    // the very start is still the option, so its `: ` is not an intro split.
    for (one, two) in [
        ("1 ", "2 "),
        ("1.", "2."),
        ("1)", "2)"),
        ("a ", "b "),
        ("a.", "b."),
        ("a)", "b)"),
    ] {
        let rows = card_rows(&format!("{one}keep: fast | {two}wait"));
        assert_eq!(rows, [format!("  {one}keep: fast"), format!("  {two}wait")]);
    }
}

#[test]
fn an_option_label_after_a_colon_space_opens_the_options_of_a_decision_card() {
    // `Next: pick: 1 keep | 2 wait`: the label follows `: `, so the options start
    // there and the intro is still split at its own `: `.
    for (one, two) in [("1 ", "2 "), ("a)", "b)")] {
        let rows = card_rows(&format!("Next: pick: {one}keep | {two}wait"));
        let want = [
            "  Next:",
            "  pick:",
            &format!("  {one}keep"),
            &format!("  {two}wait"),
        ];
        assert_eq!(rows, want);
    }
}
