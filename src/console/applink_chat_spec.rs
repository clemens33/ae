//! Frozen acceptance cases: applink L1, chat side — the link scanner and the
//! dressed chat. Independent navigator applink-nav (Z.AI GLM 5.3 Flash),
//! frozen against driver applink's plan r2 and lead's spec-r1 review. Driver
//! installs byte-identically, wired from src/console.rs via
//! `#[cfg(test)] #[path = "applink_chat_spec.rs"] mod applink_chat_spec;`.
//!
//! Level 1 NOW: a web URL or a repo-style path in a lane body becomes a real
//! terminal hyperlink (OSC 8) in `ae chat`'s dressed output (and in `ae app`'s
//! chat column — the app side is `src/app/applink_spec.rs`), so a terminal's
//! Cmd/Ctrl-click opens it. Level 2 — following a link inside ae (reader pane,
//! memo, request threads) — is a separate slice and stays out.
//!
//! What becomes a link (the human ask, chat 2026-10-19): web URLs
//! (`https://…`, `http://…`) and repo-style paths: the prefixes `src/`,
//! `docs/`, `tests/`, `.local/` (relative, resolved against the session's
//! recorded work dir), `/…` (absolute, at least two non-empty segments) and
//! `~/…` (home), each with an optional `:line` and `:line:col`. Everything
//! else — bare words, `memo <topic>`, request ids — stays text.
//!
//! The scanner's grammar, pinned here (extend only with a ruling):
//! - A token starts at text start, after Unicode whitespace, or after one of
//!   `( [ { < " ' * =` or a backtick. A control character (`char::is_control`)
//!   ends any token and permits no new one.
//! - Web: ASCII-case-insensitive `http://` / `https://` plus at least one
//!   character; ends at whitespace, a control, `<`, `>` or `"`; a trailing run
//!   of `. , ; : ! ? ' "` is stripped and so is each unbalanced closing
//!   `) ] }` (balanced pairs inside stay). The URL keeps the written bytes;
//!   ASCII `0x21..=0x7E` passes except `" < > \ ^ { | }` and a backtick,
//!   which are percent-encoded; `%` stays when two hex digits follow, else
//!   becomes `%25`; non-ASCII UTF-8 bytes are percent-encoded upper-hex.
//! - File: a token whose head is one of the path prefixes. Path characters
//!   are Unicode alphanumerics or one of `. _ - / + @ ~ # %` (the `%` ruling:
//!   kept in the candidate, `%25` in the URL). `src/…`-style relative
//!   candidates need at least one character after the prefix and resolve
//!   against `Base::dir` (absent, empty or relative: no link); `~/…` resolves
//!   against `Base::home` (absent: no link); `/…` needs at least two
//!   non-empty segments, so `/open`, `/close`, `/more`, `and/or`, `1/2` and a
//!   bare `/` never link. Optional `:N` and `:N:M` (digits) belong to the
//!   VISIBLE text, never to the URL. A trailing `.`, `,` or `;` is stripped.
//!   The file URL is `file://` with an EMPTY host and the absolute path; every
//!   byte outside `A-Za-z0-9-._~/` is percent-encoded upper-hex. No
//!   filesystem check, no `..` normalisation.
//! - `link::wrapped` wraps exactly as `console::wrap::wrap_in` does and
//!   carries the links along: a link cut by a wrap yields one link per row,
//!   same URL, each part inside its row's piece.
//!
//! Every emitted URL passes `link::is_safe_url`: printable ASCII only, no
//! control, no whitespace, no non-ASCII — the emitters' last gate.
//!
//! The chat's base is filled in production, not by hand: `Console::link_base`
//! reads the meta `work_dir` beside the journal and the console's home, and
//! `dress` takes it as a parameter so `run()` cannot dress without naming it.
//! The `run()` call itself is guarded by a source-level pin below (an in-process
//! test cannot dress: stdout is no terminal there, and a `tty` or look
//! injected by hand would no longer be the production path).
//!
//! Named residuals (accepted here, not closed): no filesystem check at draw
//! time — a shape-qualified path links whether or not it exists, and a click
//! opens whatever the text names (the terminal decides); `..` is kept,
//! un-normalised; a path whose text holds whitespace never links; tmux
//! forwards a link only when the user's own `terminal-features` knows their
//! terminal supports hyperlinks — ae never writes that server option, and a
//! tmux that strips the sequence leaves the visible text unchanged.

use std::path::Path;

use crate::console::lane::{Item, Kind, Lane};
use crate::console::link::{self, Base};
use crate::console::view::{Printed, Style};
use crate::console::wrap::wrap_in;
use crate::theme::Look;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn said(body: &str) -> Item {
    Item {
        micros: 1_759_500_000_000_000,
        kind: Kind::Said {
            who: "lead".to_owned(),
        },
        body: body.to_owned(),
        record: None,
    }
}

fn lane_of(body: &str) -> Lane {
    Lane {
        items: vec![said(body)],
        coverage: vec![],
    }
}

fn base(dir: &str) -> Base {
    Base {
        dir: Some(dir.to_owned()),
        home: Some("/h".to_owned()),
    }
}

/// The OSC 8 hyperlinks a styled output carries: `(open, url)` pairs in
/// stream order; the close carries no URL and is not listed.
fn osc8(output: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = output;
    while let Some(at) = rest.find("\x1b]8;") {
        let tail = &rest[at + 4..];
        let Some(end) = tail.find("\x1b\\") else {
            break;
        };
        let body = &tail[..end];
        if let Some(sep) = body.find(';') {
            let (params, url) = body.split_at(sep);
            let url = &url[1..];
            if !url.is_empty() {
                out.push((format!("\x1b]8;{params};{url}\x1b\\"), url.to_owned()));
            }
        }
        rest = &tail[end + 2..];
    }
    out
}

/// The OSC 8 close, byte for byte.
const CLOSE: &str = "\x1b]8;;\x1b\\";

/// `output` without the escapes ae writes on purpose: every SGR and every OSC
/// 8 pair is removed, so what remains must be plain visible text.
fn stripped(output: &str) -> String {
    let mut out = String::new();
    let mut chars = output.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => loop {
                match chars.next() {
                    Some('\x07') | None => break,
                    Some('\x1b') => {
                        if chars.next() == Some('\\') {
                            break;
                        }
                    }
                    Some(_) => {}
                }
            },
            _ => out.push('\u{fffd}'),
        }
    }
    out
}

/// The `(visible, url)` pairs a scan of `text` finds.
fn links_of(text: &str, the_base: &Base) -> Vec<(String, String)> {
    link::scan(text, the_base)
        .into_iter()
        .map(|link| (text[link.start..link.end].to_owned(), link.url))
        .collect()
}

/// An expectation pair as the scan answers it: owned strings.
fn pair(visible: &str, url: &str) -> (String, String) {
    (visible.to_owned(), url.to_owned())
}

/// The dressed chat render of one `Said` turn.
fn dressed(body: &str, the_base: Option<Base>, width: Option<usize>) -> String {
    let style = match the_base {
        Some(the_base) => {
            Style::resolve(true, Some(Look::DEFAULT), None, "lead").with_base(the_base)
        }
        None => Style::resolve(true, Some(Look::DEFAULT), None, "lead"),
    };
    let mut printed = Printed::styled(style);
    printed.set_width(width);
    printed.step(&lane_of(body), 0, true)
}

fn printable(url: &str) -> bool {
    url.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

fn words(text: &str) -> Vec<&str> {
    // The bar glyph and the option separator are ae's own dressing, never a
    // body word.
    text.split_whitespace()
        .filter(|word| !matches!(*word, "|" | "▌"))
        .collect()
}

// ---------------------------------------------------------------------------
// the scanner: what becomes a link
// ---------------------------------------------------------------------------

#[test]
fn a_web_url_becomes_a_link_with_the_written_bytes_as_its_url() {
    assert_eq!(
        links_of("see https://example.com/x now", &Base::default()),
        [pair("https://example.com/x", "https://example.com/x")],
    );
    assert_eq!(
        links_of("http://example.com/a", &Base::default()),
        [pair("http://example.com/a", "http://example.com/a")],
    );
}

#[test]
fn trailing_sentence_punctuation_stays_out_of_a_url() {
    assert_eq!(
        links_of("visit https://example.com/a, then", &Base::default()),
        [pair("https://example.com/a", "https://example.com/a")],
    );
    assert_eq!(
        links_of("it ends https://example.com/a.", &Base::default()),
        [pair("https://example.com/a", "https://example.com/a")],
    );
    assert_eq!(
        links_of("(see https://example.com/a)", &Base::default()),
        [pair("https://example.com/a", "https://example.com/a")],
    );
}

#[test]
fn a_url_with_balanced_parens_keeps_them() {
    let url = "https://en.wikipedia.org/wiki/Rust_(programming_language)";
    assert_eq!(
        links_of(
            "read https://en.wikipedia.org/wiki/Rust_(programming_language) first",
            &Base::default(),
        ),
        [pair(url, url)],
    );
}

#[test]
fn the_scheme_matches_case_insensitively_and_the_url_keeps_the_written_case() {
    assert_eq!(
        links_of("HTTPS://Example.COM/X", &Base::default()),
        [pair("HTTPS://Example.COM/X", "HTTPS://Example.COM/X")],
    );
}

#[test]
fn a_url_starts_only_after_whitespace_text_start_or_a_punctuated_edge() {
    assert!(
        links_of("xhttps://a.b", &Base::default()).is_empty(),
        "no link inside a word",
    );
    assert!(
        links_of("see,https://a.b", &Base::default()).is_empty(),
        "a comma permits no start",
    );
    assert!(
        links_of("go https:// now", &Base::default()).is_empty(),
        "a bare scheme is no link",
    );
    assert_eq!(
        links_of("url=https://a.b", &Base::default()),
        [pair("https://a.b", "https://a.b")],
    );
    assert_eq!(
        links_of("*https://a.b", &Base::default()),
        [pair("https://a.b", "https://a.b")],
    );
    assert_eq!(
        links_of("(https://a.b)", &Base::default()),
        [pair("https://a.b", "https://a.b")],
    );
}

#[test]
fn a_url_ends_at_whitespace_control_and_the_quote_and_angle_family() {
    assert_eq!(
        links_of("a https://x.y/z\"b c", &Base::default()),
        [pair("https://x.y/z", "https://x.y/z")],
    );
    assert_eq!(
        links_of("a <https://x.y/z> b", &Base::default()),
        [pair("https://x.y/z", "https://x.y/z")],
    );
    assert_eq!(
        links_of("A | https://a.b|c", &Base::default()),
        [pair("https://a.b|c", "https://a.b%7Cc")],
    );
}

#[test]
fn a_url_keeps_a_port_and_percent_encodes_what_it_may_not_carry_raw() {
    assert_eq!(
        links_of("https://example.com:8080/x", &Base::default()),
        [pair(
            "https://example.com:8080/x",
            "https://example.com:8080/x"
        )],
    );
    assert_eq!(
        links_of("see https://example.com/\u{fc}", &Base::default()),
        [pair(
            "https://example.com/\u{fc}",
            "https://example.com/%C3%BC"
        )],
    );
    assert_eq!(
        links_of("page https://x.y/a%20b#c end", &Base::default()),
        [pair("https://x.y/a%20b#c", "https://x.y/a%20b#c")],
    );
    assert_eq!(
        links_of("odd https://x.y/10% end", &Base::default()),
        [pair("https://x.y/10%", "https://x.y/10%25")],
    );
}

#[test]
fn a_repo_path_links_against_the_work_dir_its_line_stays_visible() {
    assert_eq!(
        links_of("read src/app.rs:12 first", &base("/w")),
        [pair("src/app.rs:12", "file:///w/src/app.rs")],
    );
    assert_eq!(
        links_of("src/app.rs:12:3 here", &base("/w")),
        [pair("src/app.rs:12:3", "file:///w/src/app.rs")],
    );
    assert_eq!(
        links_of("docs/x.md", &base("/w")),
        [pair("docs/x.md", "file:///w/docs/x.md")],
    );
    assert_eq!(
        links_of("evidence .local/x/REPORT.md here", &base("/w")),
        [pair(".local/x/REPORT.md", "file:///w/.local/x/REPORT.md")],
    );
}

#[test]
fn a_path_candidate_ends_at_the_colon_unless_digits_follow() {
    assert_eq!(
        links_of("the src/app.rs:x case", &base("/w")),
        [pair("src/app.rs", "file:///w/src/app.rs")],
    );
    assert_eq!(
        links_of("huge docs/x.md:99999999999999999999 end", &base("/w")),
        [pair(
            "docs/x.md:99999999999999999999",
            "file:///w/docs/x.md"
        )],
    );
}

#[test]
fn a_path_strips_its_trailing_sentence_punctuation() {
    assert_eq!(
        links_of("see src/app.rs.", &base("/w")),
        [pair("src/app.rs", "file:///w/src/app.rs")],
    );
    assert_eq!(
        links_of("then docs/x.md, and stop", &base("/w")),
        [pair("docs/x.md", "file:///w/docs/x.md")],
    );
}

#[test]
fn only_the_repo_prefixes_and_absolute_and_home_paths_link() {
    assert!(
        links_of("and/or waits", &base("/w")).is_empty(),
        "no prefix, no link",
    );
    assert!(
        links_of("the contrib/x.md dir", &base("/w")).is_empty(),
        "an unlisted prefix is no link",
    );
    assert_eq!(
        links_of("a dotfile src/.gitignore", &base("/w")),
        [pair("src/.gitignore", "file:///w/src/.gitignore")],
    );
    assert_eq!(
        links_of("a bare src/https://x.md token", &base("/w")),
        [pair("src/https", "file:///w/src/https")],
    );
    assert!(
        links_of("the half 1/2 is prose", &base("/w")).is_empty(),
        "no link expected",
    );
}

#[test]
fn an_absolute_path_needs_two_segments_and_links_without_the_work_dir() {
    assert_eq!(
        links_of("the log /Users/x/y.md holds it", &Base::default()),
        [pair("/Users/x/y.md", "file:///Users/x/y.md")],
    );
    assert!(
        links_of("run /open first", &Base::default()).is_empty(),
        "a chat command is no link",
    );
    assert!(
        links_of("bare / slash", &Base::default()).is_empty(),
        "a bare slash is no link",
    );
    assert_eq!(
        links_of("deep /Users/x/y.md:7 says", &Base::default()),
        [pair("/Users/x/y.md:7", "file:///Users/x/y.md")],
    );
}

#[test]
fn a_tilde_needs_a_slash_and_home_and_a_relative_needs_a_work_dir() {
    assert!(
        links_of("~x.md", &base("/w")).is_empty(),
        "a tilde without a slash is no home path",
    );
    assert!(
        links_of("~/notes.md", &Base::default()).is_empty(),
        "no home, no home path",
    );
    assert_eq!(
        links_of("your ~/notes.md", &base("/w")),
        [pair("~/notes.md", "file:///h/notes.md")],
    );
    assert!(
        links_of("docs/x.md", &Base::default()).is_empty(),
        "no work dir, no relative path",
    );
    assert!(
        links_of("docs/x.md", &base("")).is_empty(),
        "an empty dir is no base",
    );
    assert!(
        links_of(
            "docs/x.md",
            &Base {
                dir: Some("w".to_owned()),
                home: Some("/h".to_owned()),
            },
        )
        .is_empty(),
        "a relative dir is no base",
    );
}

#[test]
fn a_trailing_slash_on_the_dir_joins_with_one_slash() {
    assert_eq!(
        links_of("docs/x.md", &base("/w/")),
        [pair("docs/x.md", "file:///w/docs/x.md")],
    );
}

#[test]
fn path_bytes_outside_a_file_url_are_percent_encoded() {
    assert_eq!(
        links_of("docs/\u{fc}.md", &base("/w")),
        [pair("docs/\u{fc}.md", "file:///w/docs/%C3%BC.md")],
    );
    assert_eq!(
        links_of("docs/a#b.md", &base("/w")),
        [pair("docs/a#b.md", "file:///w/docs/a%23b.md")],
    );
    assert_eq!(
        links_of("the docs/100%.md share", &base("/w")),
        [pair("docs/100%.md", "file:///w/docs/100%25.md")],
    );
    assert_eq!(
        links_of("plus src/a+b.md", &base("/w")),
        [pair("src/a+b.md", "file:///w/src/a%2Bb.md")],
    );
}

#[test]
fn a_control_ends_a_token_and_permits_no_new_one() {
    // The raw path (a control stops the run mid-URL; the restart after it is
    // no token start).
    assert_eq!(
        links_of("https://exa\x07mple.com/x", &Base::default()),
        [pair("https://exa", "https://exa")],
    );
    assert!(
        links_of("x\x07https://a.b", &Base::default()).is_empty(),
        "a control permits no new token",
    );
    // The shipping path: the body was neutralised first, so the control is
    // U+FFFD inside the token, percent-encoded in the URL.
    assert_eq!(
        links_of(
            &crate::board::terminal_text("https://exa\x07mple.com/x"),
            &Base::default(),
        ),
        [pair(
            "https://exa\u{fffd}mple.com/x",
            "https://exa%EF%BF%BDmple.com/x",
        )],
    );
}

#[test]
fn a_unicode_whitespace_starts_and_ends_a_token_like_a_space() {
    assert_eq!(
        links_of("a\u{2028}https://x.y/z", &Base::default()),
        [pair("https://x.y/z", "https://x.y/z")],
    );
}

#[test]
fn every_emitted_url_is_printable_ascii() {
    let the_base = base("/w");
    for text in [
        "see https://example.com/\u{fc}?q=1 and docs/100%.md:3",
        "a\x1b]8;;https://evil\x07b https://real.example/x",
        "docs/ü.md, /Users/x/y.md:7, ~/notes.md",
        "\u{2028}https://x.y/[a](b){c}",
    ] {
        for (_, url) in links_of(text, &the_base) {
            assert!(printable(&url), "{text:?} -> {url:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// link::wrapped: the wrap measure carries the links, byte-equal to wrap_in
// ---------------------------------------------------------------------------

#[test]
fn wrapped_pieces_are_exactly_wrap_ins_pieces() {
    for text in [
        "see https://example.com/x now",
        "check https://example.invalid/abcdefghijklmnopqrst exactly",
        "alpha beta gamma delta epsilon",
        "docs/x.md:12 and /Users/x/y.md",
        "a\tb c",
        "",
    ] {
        for width in [3usize, 12, 24, 40, 200] {
            for (first, bar) in [(0usize, 0usize), (2, 1), (1, 1)] {
                let rows = link::wrapped(text, width, first, bar, &base("/w"));
                let plain = wrap_in(text, width, first, bar);
                assert_eq!(
                    rows.iter()
                        .map(|row| (row.gap, row.piece.as_str()))
                        .collect::<Vec<_>>(),
                    plain
                        .iter()
                        .map(|(gap, piece)| (*gap, piece.as_str()))
                        .collect::<Vec<_>>(),
                    "{text:?} at {width}/{first}/{bar}"
                );
            }
        }
    }
}

#[test]
fn a_link_cut_by_a_wrap_yields_one_link_per_row_with_the_same_url() {
    let rows = link::wrapped(
        "check https://example.invalid/abcdefghijklmnopqrst exactly",
        24,
        2,
        1,
        &Base::default(),
    );
    let whole = "https://example.invalid/abcdefghijklmnopqrst";
    let mut parts = Vec::new();
    for row in &rows {
        for link in &row.links {
            assert_eq!(link.url, whole);
            let part = &row.piece[link.start..link.end];
            assert!(!part.is_empty(), "a link part is never empty");
            assert!(
                !part.chars().any(char::is_whitespace),
                "a link part holds no whitespace: {part:?}"
            );
            parts.push(part.to_owned());
        }
    }
    assert!(parts.len() >= 2, "the wrap splits the link");
    assert_eq!(parts.concat(), whole, "the parts spell the one URL");
}

#[test]
fn is_safe_url_gates_the_emitters_last() {
    assert!(link::is_safe_url("https://example.com/x%20y"));
    assert!(!link::is_safe_url("https://example.com/a b"));
    assert!(!link::is_safe_url("https://example.com/\u{fc}"));
    assert!(!link::is_safe_url("https://example.com/a\x07b"));
    assert!(!link::is_safe_url("https://example.com/a\x1b\\b"));
}

// ---------------------------------------------------------------------------
// the chat: dressed output carries OSC 8 + underline, plain output neither
// ---------------------------------------------------------------------------

#[test]
fn a_dressed_chat_wraps_a_url_in_osc8_around_exactly_its_text() {
    let out = dressed("see https://example.com/x now", None, Some(200));
    let links = osc8(&out);
    assert_eq!(links.len(), 1, "{out:?}");
    assert_eq!(links[0].1, "https://example.com/x");
    let (open, url) = &links[0];
    assert!(
        out.contains(&format!("{open}\x1b[4m{url}\x1b[24m{CLOSE}")),
        "the open, the underlined text and the close stand together: {out:?}"
    );
    // The visible row is what the wrap measure says it is: the OSC 8 and SGR
    // bytes are zero-width and ride outside it.
    let rows = wrap_in("see https://example.com/x now", 200, 2, 1);
    let bare = stripped(&out);
    // A turn's render ends on a blank line; the body row is the last non-blank.
    let body = bare
        .lines()
        .rfind(|line| !line.is_empty())
        .unwrap_or_default();
    assert_eq!(body, format!("▌  {}", rows[0].1), "{out:?}");
}

#[test]
fn a_dressed_chat_wraps_a_path_in_a_file_url() {
    let out = dressed("read src/app.rs:12 first", Some(base("/w")), Some(200));
    let links = osc8(&out);
    assert_eq!(links.len(), 1, "{out:?}");
    assert_eq!(links[0].1, "file:///w/src/app.rs");
    let bare = stripped(&out);
    assert!(bare.contains("src/app.rs:12"), "{out:?}");
    assert!(!bare.contains('%'), "the visible text is not rewritten");
}

#[test]
fn a_dressed_chat_without_a_width_still_links() {
    let out = dressed("see https://example.com/x now", None, None);
    assert_eq!(osc8(&out).len(), 1, "{out:?}");
}

#[test]
fn plain_chat_output_carries_no_osc8_and_no_underline() {
    let body = "see https://example.com/x and src/app.rs:12";
    let mut printed = Printed::styled(Style::PLAIN);
    printed.set_width(Some(200));
    let out = printed.step(&lane_of(body), 0, true);
    assert!(!out.contains("\x1b]8;"), "{out:?}");
    assert!(!out.contains("\x1b[4m"), "{out:?}");
    let dressed_out = dressed(body, Some(base("/w")), Some(200));
    assert!(osc8(&dressed_out).len() >= 2, "{dressed_out:?}");
}

#[test]
fn a_hostile_body_never_smuggles_an_escape_and_never_links_unsafe() {
    let body = "a\x1b]8;;https://evil\x07b https://real.example/x";
    let out = dressed(body, Some(base("/w")), Some(200));
    let bare = stripped(&out);
    assert!(
        bare.chars()
            .filter(|ch| *ch != '\n')
            .all(|ch| !ch.is_control()),
        "a control survived the strip: {bare:?}"
    );
    assert!(
        bare.contains("]8;;https://evil"),
        "the smuggled sequence shows as inert text: {bare:?}"
    );
    for (_, url) in osc8(&out) {
        assert!(printable(&url), "{url:?}");
    }
    assert_eq!(
        words(
            bare.lines()
                .rfind(|line| !line.is_empty())
                .unwrap_or_default()
        ),
        words(&crate::board::terminal_text(body)),
        "the visible words are the neutralised body's words"
    );
}

#[test]
fn a_url_split_by_a_wrap_is_two_links_with_one_id() {
    let body = "check https://example.invalid/abcdefghijklmnopqrst exactly";
    let out = dressed(body, None, Some(24));
    let links = osc8(&out);
    let whole = "https://example.invalid/abcdefghijklmnopqrst";
    let parts: Vec<&(String, String)> = links.iter().filter(|(_, url)| url == whole).collect();
    assert!(parts.len() >= 2, "the wrap splits the link: {out:?}");
    let opens: Vec<&str> = parts.iter().map(|(open, _)| open.as_str()).collect();
    assert!(
        opens.windows(2).all(|pair| pair[0] == pair[1]),
        "the split parts share one open, id included: {opens:?}"
    );
    // The parts are slices of the one visible URL, in order. The search runs
    // on the RAW output: the opens exist only there, not in the stripped text.
    let mut joined = String::new();
    let mut rest = out.as_str();
    for open in &opens {
        let Some(at) = rest.find(open) else {
            panic!("open {open:?} not in {rest:?}");
        };
        let after = &rest[at + open.len()..];
        let Some(end) = after.find(CLOSE) else {
            panic!("close not after {open:?}");
        };
        let seg = &after[..end];
        let part = seg.strip_prefix("\x1b[4m").unwrap_or(seg);
        let part = part.strip_suffix("\x1b[24m").unwrap_or(part);
        joined.push_str(part);
        rest = &after[end + CLOSE.len()..];
    }
    assert_eq!(joined, whole, "the parts spell the one URL: {out:?}");
}

#[test]
fn fold_counts_do_not_move_when_a_body_carries_links() {
    let body = |extra: &str| {
        (1..=30)
            .map(|at| format!("l{at}"))
            .chain([extra.to_owned()])
            .collect::<Vec<_>>()
            .join("\n")
    };
    let fold_row = |body: &str| {
        let mut printed = Printed::styled(
            Style::resolve(true, Some(Look::DEFAULT), None, "lead").with_base(base("/w")),
        );
        printed.set_width(Some(40));
        printed.set_fold(true);
        let out = printed.step(&lane_of(body), 0, true);
        stripped(&out)
            .lines()
            .position(|line| line.contains("more lines"))
    };
    let plain = fold_row(&body("plain tail"));
    let linked = fold_row(&body("see https://example.invalid/abcdefghijklmnopqrst"));
    assert_eq!(plain, linked, "the fold row stands where it stood");
    assert!(plain.is_some(), "the turn folds at all");
}

#[test]
fn an_evidence_row_links_its_path_too() {
    let body = "Pick: vertical wins | horizontal waits.\n\ndetails: .local/x.md";
    let out = dressed(body, Some(base("/w")), Some(200));
    assert!(
        osc8(&out)
            .iter()
            .any(|(_, url)| url == "file:///w/.local/x.md"),
        "{out:?}"
    );
}

#[test]
fn an_option_row_links_its_words_too() {
    let body = "Pick: go to https://docs.example/start | wait for the freeze";
    let out = dressed(body, None, Some(200));
    assert!(
        osc8(&out)
            .iter()
            .any(|(_, url)| url == "https://docs.example/start"),
        "{out:?}"
    );
}

// ---------------------------------------------------------------------------
// the chat's production seams: the base is filled, not handed in
// ---------------------------------------------------------------------------

#[test]
fn the_console_reads_its_link_base_from_the_meta_work_dir_and_its_home() {
    // A scratch session dir of this test alone, removed before any assert, so
    // no path through the test can leak it.
    let root = std::env::temp_dir().join(format!(
        "ae-applink-chat-spec-console-base-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the scratch session dir creates");
    std::fs::write(root.join("meta"), b"seat.main=lead\nwork_dir=/w\n")
        .expect("the scratch meta writes");
    let linked = {
        let console = crate::console::Console::open("applink-spec".to_owned(), root.clone());
        let mut console = console;
        console.home = Some(Path::new("/h").to_owned());
        console.link_base()
    };
    // The bare case is the SAME dir with the work_dir row gone.
    std::fs::write(root.join("meta"), b"seat.main=lead\n").expect("the bare meta writes");
    let bare = {
        let console = crate::console::Console::open("applink-spec".to_owned(), root.clone());
        let mut console = console;
        console.home = Some(Path::new("/h").to_owned());
        console.link_base()
    };
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(
        linked,
        Base {
            dir: Some("/w".to_owned()),
            home: Some("/h".to_owned()),
        },
    );
    assert_eq!(
        bare,
        Base {
            dir: None,
            home: Some("/h".to_owned()),
        },
        "a meta without work_dir names no dir",
    );
}

/// Source-level guard (an in-process test cannot dress: stdout is no terminal
/// there): `run()` must dress through the console's link base, and dress must
/// take the base, so no code path can build the chat's Style without naming
/// where its paths resolve.
#[test]
fn run_dresses_the_console_through_its_link_base() {
    let source = include_str!("../console.rs");
    assert!(
        source.contains("console.link_base()"),
        "run() must pass the console's link base to dress"
    );
    assert!(
        source.contains("fn dress(name: &str, seats: &[Seat], base:"),
        "dress must take the base parameter"
    );
}
