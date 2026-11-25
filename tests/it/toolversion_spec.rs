//! `ToolKind::measured_version` is the newest release of a harness that a frozen
//! fixture proves, and the fixture FILE NAMES are the oracle: a capture is named
//! `<tool>-<words>-<release>-<words>.<ext>` (`claude-2.1.330-hooks-80x24.txt`,
//! `codex-service-wait-0.170.0-80w.txt`, `agy-quota-1.2.14.json`), so the pin is
//! read from the tree, never from the constant it checks. `ae list` and `ae doctor`
//! draw that constant beside a seat's measured release, so a stale constant says
//! "measured 2.1.330" for a release no fixture holds, and a fixture added without
//! the constant moving leaves the cell behind the evidence.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the fixture walks the checked-in capture tree"
)]

use ae::tool::ToolKind;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// `(tool word, kind)` for every harness ae models that has a fixture name shape.
const TOOLS: [(&str, ToolKind); 6] = [
    ("claude", ToolKind::Claude),
    ("codex", ToolKind::Codex),
    ("agy", ToolKind::Agy),
    ("grok", ToolKind::Grok),
    ("muse", ToolKind::Muse),
    ("opencode", ToolKind::OpenCode),
];

/// Every tool word a name may start with, `gemini` too: ae models it and no fixture
/// names a release for it, which the second test pins.
const WORDS: [&str; 7] = [
    "claude", "codex", "gemini", "agy", "grok", "muse", "opencode",
];

type Release = [u32; 3];

/// The release a `-`/`_`-delimited name segment spells: `N.N.N` of one to nine
/// digits a part, then the end of the segment or `.` and an extension that does
/// not start with a digit. `2.1.330`, `0.170.0` and `1.2.14.json` are releases;
/// `2.1.330foo`, `2.1.3.4`, `2.1` and `80w.txt` are not.
fn release_segment(segment: &str) -> Option<Release> {
    let mut rest = segment;
    let mut found = [0_u32; 3];
    for (at, part) in found.iter_mut().enumerate() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=9).contains(&digits) {
            return None;
        }
        *part = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        if at < 2 {
            rest = rest.strip_prefix('.')?;
        }
    }
    match rest.strip_prefix('.') {
        None => rest.is_empty().then_some(found),
        Some(extension) => extension
            .chars()
            .next()
            .is_some_and(|first| !first.is_ascii_digit())
            .then_some(found),
    }
}

/// `(tool, newest release the name spells)` when `name` starts `<tool>-` and holds
/// a release segment anywhere after it.
fn named(name: &str) -> Option<(&'static str, Release)> {
    let tool = WORDS.into_iter().find(|tool| {
        name.strip_prefix(*tool)
            .is_some_and(|rest| rest.starts_with('-'))
    })?;
    name.split(['-', '_'])
        .filter_map(release_segment)
        .max()
        .map(|release| (tool, release))
}

fn walk(dir: &Path, found: &mut BTreeMap<&'static str, BTreeSet<Release>>) {
    for entry in std::fs::read_dir(dir).expect("fixture directory") {
        let entry = entry.expect("fixture entry");
        if let Some((tool, release)) = entry.file_name().to_str().and_then(named) {
            found.entry(tool).or_default().insert(release);
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            walk(&entry.path(), found);
        }
    }
}

fn releases_by_fixture_name() -> BTreeMap<&'static str, BTreeSet<Release>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut found = BTreeMap::new();
    walk(&root, &mut found);
    found
}

fn spelled(release: Release) -> String {
    format!("{}.{}.{}", release[0], release[1], release[2])
}

#[test]
fn the_measured_release_of_each_tool_is_the_newest_its_fixture_names_prove() {
    let found = releases_by_fixture_name();
    // Control: a walk that found nothing, or only the one name shape, would pass
    // every comparison below vacuously. These older captures carry their release
    // in the MIDDLE of the name, so seeing them proves that shape is read.
    for (tool, older) in [
        ("claude", [2, 1, 317]),
        ("codex", [0, 170, 0]),
        ("agy", [1, 2, 14]),
    ] {
        assert!(
            found.get(tool).is_some_and(|all| all.contains(&older)),
            "{tool}: the walk did not see {older:?}: {found:?}"
        );
    }
    for (word, kind) in TOOLS {
        assert_eq!(
            kind.measured_version().map(str::to_owned),
            found
                .get(word)
                .and_then(|all| all.iter().next_back().copied())
                .map(spelled),
            "{word}: the adapter's measured release is not the newest fixture name"
        );
    }
}

#[test]
fn a_tool_no_fixture_names_has_no_measured_release() {
    let found = releases_by_fixture_name();
    assert!(
        !found.contains_key("gemini"),
        "a gemini fixture now exists: {found:?}"
    );
    assert_eq!(ToolKind::Gemini.measured_version(), None);
    assert_eq!(ToolKind::Unknown.measured_version(), None);
}

#[test]
fn the_name_reader_takes_a_release_anywhere_after_the_tool_and_nothing_else() {
    for (name, want) in [
        ("claude-2.1.330-hooks-80x24.txt", ("claude", [2, 1, 330])),
        ("claude-idle-2.1.330.esc", ("claude", [2, 1, 330])),
        ("claude-limit-echo-2.1.317.txt", ("claude", [2, 1, 317])),
        ("codex-0.171.0-login-120x40.txt", ("codex", [0, 171, 0])),
        ("codex-service-wait-0.170.0-80w.esc", ("codex", [0, 170, 0])),
        ("agy-quota-1.2.14.json", ("agy", [1, 2, 14])),
        (
            "claude-2.1.330-mcp40-80x24.history.txt",
            ("claude", [2, 1, 330]),
        ),
    ] {
        assert_eq!(named(name), Some(want), "{name}");
    }
    assert!(named("claude-2.1.99-x.txt") < named("claude-2.1.330-x.txt"));
    for name in [
        // No tool word first: an orchestrator capture names ae's release.
        "v2026.9.9.config",
        "orchestrator-v2026.9.9.config",
        "claude-composer",
        "claude-2.1-x.txt",
        "claude-2.1.3.4-x.txt",
        "claude-2.1.330foo.txt",
        "claude-2.1.330x-hooks.txt",
        "claude2.1.330.txt",
        "claudex-2.1.330.txt",
        "opencode-v2",
        "the-claude-2.1.330.txt",
    ] {
        assert_eq!(named(name), None, "{name}");
    }
}
