//! cliparse supplement: the unknown-flag-then-extra shapes the frozen
//! `cliparse_spec` left unpinned (mutation survivor `src/cli.rs:1299:40`,
//! `parse_compact_revalidate`'s `--keep-history` guard forced true).
//!
//! Each helper whose grammar names "the first unexpected token" has a guarded
//! arm for `<operands> <valid-flag> <extra>`; an UNKNOWN flag in that position
//! must fall through to the generic arm and be named itself. The oracle is the
//! baseline's observable result (73c6b309), in the frozen spec's golden format.

use ae::cli::Request;

/// One argv and its golden.
type Case = (&'static [&'static str], &'static str);

const FLAG_EXTRA: &[Case] = &[
    (
        &["_compact-revalidate", "/d", "t", "--bogus", "x"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-revalidate", "/d", "t", "--bogus", "L", "x"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-teardown", "/d", "t", "--bogus", "x"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-wait", "/d", "ref", "--bogus", "9", "x"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-freeze", "/d", "--bogus", "x"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_end-nonlocal-teardown", "/d", "--bogus", "x"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &[
            "_compact-archive",
            "/d",
            "t",
            "at",
            "o",
            "r",
            "p",
            "w",
            "--bogus",
            "x",
        ],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
];

#[test]
fn an_unknown_flag_before_an_extra_word_is_the_token_named() {
    let drift: Vec<String> = FLAG_EXTRA
        .iter()
        .filter_map(|(words, want)| {
            let args: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
            let request = Request::parse(&args);
            let got = format!(
                "{request:?} exit={:?} dir={:?}",
                request.exit_code(),
                request.session_dir()
            );
            (got != *want).then(|| format!("{words:?}\n   want {want}\n    got {got}"))
        })
        .collect();
    assert!(
        drift.is_empty(),
        "FLAG_EXTRA drifted:\n{}",
        drift.join("\n")
    );
}
