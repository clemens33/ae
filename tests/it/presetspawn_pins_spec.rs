//! Independent mutation pins for #296 part A.
//! Oracle: plan section 12's actionable void-row reason and control-character
//! safety. Exact escaping and the 40-character clip pin candidate 31d15477's
//! behavior; they are not an independently ruled requirement.
//! Drive the public parser and refusal, rather than the private echo helper.

use std::path::Path;

use ae::pairs::{Verdict, parse_in};

fn refusal_for(entry: &str) -> String {
    let global = Path::new("config");
    let pairs = parse_in(global, &format!("[client_pairs]\nopus = cc-mic, {entry}\n"));
    let verdict = pairs.judge("opus", "cc-mic");
    assert!(
        matches!(verdict, Verdict::Void { .. }),
        "a bad entry voids the entire row: {verdict:?}"
    );
    let Some(refusal) = verdict.refusal("opus", global) else {
        panic!("a void row must explain its refusal");
    };
    refusal
}

#[test]
fn presetspawn_bad_entry_refusal_preserves_and_escapes_the_entry() {
    for (entry, expected) in [
        ("cc mic", r#""cc mic" is not a client label"#),
        (
            "cc\t\"mic\\route",
            r#""cc\t\"mic\\route" is not a client label"#,
        ),
    ] {
        let refusal = refusal_for(entry);
        assert!(
            refusal.contains(expected),
            "the refusal must identify the escaped bad entry {expected:?}: {refusal}"
        );
        assert!(
            !refusal.chars().any(char::is_control),
            "a config control byte must never reach the terminal: {refusal:?}"
        );
    }
}

#[test]
fn presetspawn_bad_entry_refusal_clips_at_40_characters_not_bytes() {
    // 39 ASCII digits and one two-byte character; the tail starts at char 41.
    let refusal = refusal_for("123456789012345678901234567890123456789éTAIL_MUST_NOT_APPEAR");
    assert!(
        refusal.contains(r#""123456789012345678901234567890123456789é" is not a client label"#),
        "the refusal must preserve exactly the first 40 characters: {refusal}"
    );
    assert!(
        !refusal.contains("TAIL_MUST_NOT_APPEAR"),
        "the refusal must clip the rejected entry: {refusal}"
    );
}
