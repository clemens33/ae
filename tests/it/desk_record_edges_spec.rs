//! Two input-grammar boundaries agreed for the board.

use ae::desk::parse_input;

#[test]
fn a_link_can_use_all_300_bytes_and_an_item_needs_its_section_first() {
    let prefix = "https://example.test/";
    let link = format!("{prefix}{}", "x".repeat(300 - prefix.len()));
    let input = format!("DECIDE\nD7 | Follow the notes | {link}\n");
    assert_eq!(link.len(), 300);
    assert!(parse_input(input.as_bytes()).is_ok());

    let error = parse_input(b"D7 | No heading yet\nDECIDE\n");
    assert!(matches!(error, Err(why) if why.line == 1));
}
