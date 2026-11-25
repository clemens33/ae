//! Edges of the row tag window and of the DECIDE option markers, through the
//! public API of `ae web`.

use std::collections::BTreeSet;

use ae::web::chat::{TAGS_WINDOW, row_tags};
use ae::web::choice::{Choices, parse};

/// A tag that ends inside the window, with its closer, stays read when a
/// character of two bytes lies across the window's end.
#[test]
fn a_closed_tag_before_a_character_across_the_edge_is_read() {
    let known = BTreeSet::from(["calm"]);
    let head = format!("{} #calm.", "x".repeat(TAGS_WINDOW - 8));
    assert_eq!(head.len(), TAGS_WINDOW - 1);
    assert_eq!(row_tags(&format!("{head}\u{e9}a end"), &known), ["calm"]);
}

/// A longer word that the window cuts right before it would spell a known slug
/// reads as no tag.
#[test]
fn no_part_of_a_longer_word_reads_as_a_tag() {
    let known = BTreeSet::from(["calm"]);
    let head = format!("{} #cal", "x".repeat(TAGS_WINDOW - 5));
    assert_eq!(head.len(), TAGS_WINDOW);
    assert!(row_tags(&format!("{head}mer end"), &known).is_empty());
}

/// Only `1)` to `9)` mark an option, so a `0)` stays in its text.
#[test]
fn a_zero_marker_stays_in_the_question() {
    assert_eq!(
        parse("Reset to 0) first? 1) Yes 2) No"),
        Some(Choices {
            question: "Reset to 0) first?".to_owned(),
            options: vec!["Yes".to_owned(), "No".to_owned()],
            recommend: None,
        }),
    );
}
