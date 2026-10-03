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
