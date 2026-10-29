//! #270 compatibility: both ae chat surfaces retain released plain Markdown bytes.
use ae::console::{
    lane::{Item, Kind, Lane},
    view::{Printed, Style},
};
use ae::theme::Look;

#[test]
fn plain_and_dressed_chat_keep_markdown_source_bytes() {
    let body = "# head\n**raw** `code`\n- one\n- two";
    let kind = Kind::Said { who: "lead".into() };
    assert_eq!(Style::PLAIN.line(&kind, body), format!("  {body}\n"));
    let look = Look::read("on", "neutral", "on", "on");
    let style = Style::resolve(true, Some(look), None, "lead");
    assert_eq!(
        style.line(&kind, body),
        format!("\x1b[38;2;87;182;194m▌\x1b[0m  {body}\n")
    );
    let lane = Lane {
        items: vec![Item {
            micros: 1_759_500_000_000_000,
            kind,
            body: body.into(),
            record: None,
        }],
        coverage: vec![],
    };
    for style in [Style::PLAIN, style] {
        let got = Printed::styled(style).step(&lane, 0, true);
        assert!(got.contains("# head") && got.contains("**raw** `code`"));
        assert!(got.contains("- one") && got.contains("- two"));
    }
}
