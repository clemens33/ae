//! Frozen acceptance cases: appread L2, link side — `console::link::files`,
//! the file subset of the one link grammar with its targets decoded by
//! construction. Independent navigator appread-nav (Meta Muse Wonder Full
//! 1.0 contributor), frozen against driver appread's plan-r3 (lead ruling:
//! a shared walk, no percent-decoder; nav critique A1-A10 adopted). Driver
//! installs byte-identically, wired from src/console.rs via
//! `#[cfg(test)] #[path = "console/appread_link_spec.rs"] mod
//! appread_link_spec;`. The app side is `src/app/appread_spec.rs`, the
//! loader side `src/app/loader/read_spec.rs`.
//!
//! `files(text, base)` answers one `FileRef{start, end, path, line, col}`
//! per FILE link `scan` finds, in scan order: the same byte range (the
//! `:N[:M]` suffix inside `end`), `path` the absolute unencoded path the
//! link resolves to (`src/`-kin against `base.dir`, `~/` against
//! `base.home`, `..` kept as written), `line`/`col` the suffix numbers.
//! An overflowing number is `None` for that field only; the suffix stays
//! inside `end`. Web URLs never appear. No filesystem is read, so `files`
//! is total and its cross-pin is against `scan` itself: ranges equal the
//! `file://`-subset of `scan`'s ranges, and re-encoding a `path` with the
//! spec's own independent encoder spells that link's `scan` URL exactly.
//! What L1 links at all stays pinned by the applink spec beside this one;
//! this file pins only what `files` adds.

use std::fmt::Write as _;

use super::link::{Base, FileRef, files, scan};

fn base() -> Base {
    Base {
        dir: Some("/w".to_owned()),
        home: Some("/h".to_owned()),
    }
}

/// The spec's independent percent-encoder: `file://` plus the path with
/// every byte outside `A-Za-z0-9-._~/` as upper-hex `%XX`. The oracle for
/// a `FileRef`'s path is that this spells the matching `scan` URL.
fn encode(path: &str) -> String {
    let mut out = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            write!(out, "%{byte:02X}").expect("a String takes the write");
        }
    }
    out
}

/// `files` with the cross-pin against `scan` asserted on the way out.
fn checked(text: &str, the_base: &Base) -> Vec<FileRef> {
    let found = files(text, the_base);
    let subset: Vec<(usize, usize, String)> = scan(text, the_base)
        .into_iter()
        .filter(|link| link.url.starts_with("file://"))
        .map(|link| (link.start, link.end, link.url))
        .collect();
    let ranges: Vec<(usize, usize, String)> = found
        .iter()
        .map(|file| (file.start, file.end, encode(&file.path)))
        .collect();
    assert_eq!(ranges, subset, "files over {text:?}");
    for file in &found {
        assert!(text.get(file.start..file.end).is_some(), "{file:?}");
    }
    found
}

fn paths(found: &[FileRef]) -> Vec<(&str, Option<usize>, Option<usize>)> {
    found
        .iter()
        .map(|file| (file.path.as_str(), file.line, file.col))
        .collect()
}

#[test]
fn relative_prefixes_resolve_against_the_work_dir_with_line_and_col() {
    let found = checked(
        "see src/app.rs:12:5 and docs/app.md, tests/it/x.rs .local/p.md:3",
        &base(),
    );
    assert_eq!(
        paths(&found),
        [
            ("/w/src/app.rs", Some(12), Some(5)),
            ("/w/docs/app.md", None, None),
            ("/w/tests/it/x.rs", None, None),
            ("/w/.local/p.md", Some(3), None),
        ],
    );
}

#[test]
fn web_urls_never_appear_and_order_is_the_text_order() {
    let found = checked(
        "https://example.com/src/a.rs src/b.rs http://x/y docs/c.md",
        &base(),
    );
    assert_eq!(
        paths(&found),
        [("/w/src/b.rs", None, None), ("/w/docs/c.md", None, None)],
    );
    let twice = checked("src/a.rs src/a.rs", &base());
    assert_eq!(twice.len(), 2, "a repeated link is two file refs");
    assert!(twice[0].start < twice[1].start);
}

#[test]
fn home_and_absolute_paths_resolve_without_the_work_dir() {
    let found = checked("~/notes/a.txt /abs/b/c.rs:7", &base());
    assert_eq!(
        paths(&found),
        [
            ("/h/notes/a.txt", None, None),
            ("/abs/b/c.rs", Some(7), None),
        ],
    );
    let root_home = Base {
        dir: None,
        home: Some("/".to_owned()),
    };
    assert_eq!(
        paths(&checked("~/x.rs", &root_home)),
        [("/x.rs", None, None)],
    );
}

#[test]
fn an_absent_or_relative_base_dir_links_no_relative_path() {
    let no_dir = Base {
        dir: None,
        home: Some("/h".to_owned()),
    };
    assert_eq!(
        paths(&checked("src/a.rs ~/a.rs /a/b.rs", &no_dir)),
        [("/h/a.rs", None, None), ("/a/b.rs", None, None)],
    );
    let relative = Base {
        dir: Some("w".to_owned()),
        home: None,
    };
    assert!(checked("src/a.rs ~/a.rs", &relative).is_empty());
    let trailing = Base {
        dir: Some("/w/".to_owned()),
        home: None,
    };
    assert_eq!(
        paths(&checked("src/a.rs", &trailing)),
        [("/w/src/a.rs", None, None)],
    );
}

#[test]
fn dotdot_is_kept_and_a_trailing_dot_is_not_part_of_the_path() {
    let text = "src/../docs/a.md and src/b.rs.";
    let found = checked(text, &base());
    assert_eq!(
        paths(&found),
        [
            ("/w/src/../docs/a.md", None, None),
            ("/w/src/b.rs", None, None),
        ],
    );
    let second = &found[1];
    assert_eq!(&text[second.start..second.end], "src/b.rs");
}

#[test]
fn percent_and_unicode_stay_raw_in_the_path_and_encoded_in_the_url() {
    let found = checked("src/a%20b.rs src/ü.rs", &base());
    assert_eq!(
        paths(&found),
        [("/w/src/a%20b.rs", None, None), ("/w/src/ü.rs", None, None)],
    );
    // The cross-pin already proved the URLs; spell the interesting one.
    assert_eq!(encode(&found[0].path), "file:///w/src/a%2520b.rs");
}

#[test]
fn line_suffix_edges_colons_overflows_and_strays() {
    // (text, resolved path, line, col, the suffix the range holds)
    type Case = (
        &'static str,
        &'static str,
        Option<usize>,
        Option<usize>,
        &'static str,
    );
    let cases: &[Case] = &[
        ("src/a.rs:0", "/w/src/a.rs", Some(0), None, "src/a.rs:0"),
        ("src/a.rs:12:", "/w/src/a.rs", Some(12), None, ":12"),
        (
            "src/a.rs:12:34:56",
            "/w/src/a.rs",
            Some(12),
            Some(34),
            "src/a.rs:12:34",
        ),
        ("src/a.rs:1x", "/w/src/a.rs", Some(1), None, "src/a.rs:1"),
        ("src/a:b.rs", "/w/src/a", None, None, "src/a"),
        (
            "src/a.rs:99999999999999999999999999",
            "/w/src/a.rs",
            None,
            None,
            "99999999999999999999999999",
        ),
        (
            "src/a.rs:12:99999999999999999999999999",
            "/w/src/a.rs",
            Some(12),
            None,
            "src/a.rs:12:99999999999999999999999999",
        ),
    ];
    for (text, path, line, col, held) in cases {
        let found = checked(text, &base());
        assert_eq!(paths(&found), [(*path, *line, *col)], "{text:?}");
        let one = &found[0];
        assert!(
            text[one.start..one.end].contains(held),
            "{text:?} holds {held:?} inside {:?}",
            &text[one.start..one.end],
        );
    }
    // The whole visible range, pinned where the table cannot hold it.
    let text = "src/a.rs:12:34:56";
    let one = &checked(text, &base())[0];
    assert_eq!(&text[one.start..one.end], "src/a.rs:12:34");
    let text = "src/a.rs:12:";
    let one = &checked(text, &base())[0];
    assert_eq!(&text[one.start..one.end], "src/a.rs:12");
}

#[test]
fn tokens_that_link_nothing_yield_no_file_ref() {
    for text in [
        "",
        "plain words only",
        "/open and/or 1/2",
        "src/ is a dir",
        "xsrc/a.rs",
        "word:src/a.rs",
        "a.rs:12 has no prefix",
    ] {
        assert!(checked(text, &base()).is_empty(), "{text:?}");
    }
}

#[test]
fn openers_start_tokens_and_a_control_ends_one() {
    let text = "(src/a.rs) `docs/b.md`";
    let found = checked(text, &base());
    assert_eq!(
        paths(&found),
        [("/w/src/a.rs", None, None), ("/w/docs/b.md", None, None)],
    );
    assert_eq!(&text[found[0].start..found[0].end], "src/a.rs");
    let cut = checked("src/a\u{7}.rs src/b.rs", &base());
    assert_eq!(
        paths(&cut),
        [("/w/src/a", None, None), ("/w/src/b.rs", None, None)],
    );
}
