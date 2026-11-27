//! Frozen banked-resets acceptance r7: the expiry grammar accepts whole minutes
//! or whole seconds, never fractions.
//!
//! INSTALL (driver, byte-identical): copy to
//! `src/l2_model/config/banked_grammar_spec.rs` and add before the
//! `desk_template_spec` block in `src/l2_model/config.rs` the triple, as text:
//! `#[cfg(test)]`, `#[path = "config/banked_grammar_spec.rs"]`,
//! `mod banked_grammar_spec;`
//! Additive to r6 (`8969d491acf1a5010da2d53f51275a73cde5f7c4`); no earlier pin moves.
//! Oracle: brief ruling 1 (an unparsable date is refused with one note) and the
//! published grammar in docs/reference/commands.md ("Whole minutes and seconds
//! are supported"). Epochs are hand-computed: 2026-12-01T00:00:00Z is
//! `1_796_083_200` (2026-01-01 is `1_767_225_600`, plus 334 days).
//! Mutant it kills: `config.rs` `reset_expiry`, `||` to `&&` in the zone check,
//! which admits the six-byte zone `.1234Z`.

#![allow(
    clippy::expect_used,
    reason = "acceptance reads each parse result once"
)]

#[test]
fn a_fractional_second_expiry_is_refused_with_its_reason() {
    for declaration in [
        "2026-12-01T23:22:00.1234Z",
        "2026-12-01T23:22:00.5+01:00",
        "2026-12-01T23:22+01:00,2026-12-12T23:17:00.1234Z",
    ] {
        let refused = super::parse_manual_resets(declaration);
        let note = refused.expect_err(declaration);
        assert!(
            note.contains("manual_resets=") && note.contains("ignored"),
            "{declaration}: one refusal note names the key: {note}"
        );
    }
}

#[test]
fn a_whole_second_expiry_with_an_offset_counts_at_that_instant() {
    // 23:22:30 at +01:00 is 22:22:30Z: 1_796_083_200 + 80_550.
    let declared =
        super::parse_manual_resets("2026-12-01T23:22:30+01:00").expect("whole seconds are usable");
    assert_eq!(declared.expiries(), Some(&[1_796_163_750][..]));
    assert_eq!(declared.count(), 1);
}
