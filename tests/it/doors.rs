//! The DOORS — the capability boundary, asked of the tree rather than remembered.
//!
//! `clippy.toml` denies `std::process::Command` and a short list of `std::fs`
//! readers everywhere but a named few files. That deny is the boundary; these
//! are the tests that keep it honest, and they are gathered here because they
//! are about the CRATE, not about any one feature of it:
//!
//! * the boundary survives any lint relaxation, asked of clippy itself with
//!   `--force-warn` rather than of the source text;
//! * the counter that enumerates relaxation FORMS still sees the four shapes
//!   that have already beaten it, and sees exactly the ones present in the tree;
//! * `git` and `ps` each have exactly one product caller;
//! * a child process is run in exactly one place in the test harness, and is
//!   wrapped there.
//!
//! Slice Z4 moved these out of the parity harness's self-tests. The harness
//! existed to run a bash lane beside a core lane and compare the artifacts, and
//! it went with the bash it was comparing — but these guards were never about
//! that comparison. What survives of the harness is its ONE door,
//! [`super::parity`]'s `Invocation` + `capture::raw`, which is the thing the
//! last test below is about.

#![allow(
    clippy::disallowed_methods,
    reason = "these read the crate's own source and config; the capability boundary is \
              about what PRODUCT code may reach"
)]

use std::fs;
use std::ops::Range;
use std::path::Path;

use ae::json::{self, Value};

/// One of reviewer4's round-4 injections against the parity harness, byte for
/// byte.
const RAW_STATUS_JUDGEMENT: &str = "        let output = command.output()?;
        if std::env::var_os(\"AE_PARITY_STATUS_JUDGE\").is_some() && !output.status.success() {
            return Err(io::Error::other(\"lane status differed\"));
        }
";

/// Every `.rs` file in the crate, for a guard that must see the whole tree.
fn rust_sources() -> Vec<std::path::PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    walk(&root.join("src"), &mut found);
    walk(&root.join("tests"), &mut found);
    found.sort();
    assert!(
        found.len() > 5,
        "the source walk found {} files; a guard that scans nothing passes forever",
        found.len()
    );
    found
}

/// Every file clippy reports a `std::process::Command` in — asked SEMANTICALLY.
fn command_sites_reported_by_clippy() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());

    // Its own target dir: the outer test run holds the normal one, and a guard
    // that blocks on someone else's lock is a guard that times out.
    #[allow(
        clippy::disallowed_types,
        reason = "the guard's own door: it must run clippy to ask clippy anything"
    )]
    let output = std::process::Command::new(cargo)
        .current_dir(manifest)
        .args([
            "clippy",
            "--quiet",
            "--locked",
            "--all-targets",
            "--all-features",
            "--message-format=json",
        ])
        .arg("--target-dir")
        .arg(manifest.join("target").join("force-warn-guard"))
        .args(["--", "--force-warn", "clippy::disallowed_types"])
        .output()
        .unwrap_or_else(|err| panic!("this guard needs cargo and clippy on PATH: {err}"));

    assert!(
        output.status.success(),
        "clippy did not run: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut sites = Vec::new();
    for line in stdout.lines() {
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = json::parse(line) else {
            continue;
        };
        if value.get_str("reason") != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("code").and_then(|code| code.get_str("code"))
            != Some("clippy::disallowed_types")
        {
            continue;
        }
        let Some(Value::Arr(spans)) = message.get("spans") else {
            continue;
        };
        for span in spans {
            if span.get("is_primary") != Some(&Value::Bool(true)) {
                continue;
            }
            if let Some(file) = span.get_str("file_name") {
                sites.push(file.to_owned());
            }
        }
    }
    sites.sort();
    sites.dedup();
    sites
}

#[test]
fn the_capability_boundary_holds_against_any_lint_relaxation() {
    let sites = command_sites_reported_by_clippy();

    // Non-vacuity FIRST.
    assert!(
        !sites.is_empty(),
        "the force-warn probe reported no `Command` anywhere; it did not run, and \
         a guard that scans nothing passes forever"
    );

    // Asked of the compiler, so no `allow` of any shape can hide a site from it:
    // these ARE the places this crate can start a child process.
    assert_eq!(
        sites,
        vec![
            "src/install.rs".to_owned(),
            "src/run.rs".to_owned(),
            "src/transport.rs".to_owned(),
            "src/upgrade.rs".to_owned(),
            "tests/it/cli.rs".to_owned(),
            "tests/it/doors.rs".to_owned(),
            "tests/it/install.rs".to_owned(),
            "tests/it/parity.rs".to_owned(),
            "tests/it/shape.rs".to_owned(),
        ],
        "the set of places this crate can start a child process changed"
    );
}

/// `transport::run_git` is the FIXED-PROGRAM git leg of the one process door:
/// it chooses the binary (`git`) so a caller only chooses arguments.
#[test]
fn run_git_has_exactly_one_product_caller() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut holders: Vec<String> = rust_sources()
        .into_iter()
        .filter(|p| p.starts_with(root.join("src")))
        .filter(|p| fs::read_to_string(p).is_ok_and(|text| text.contains("run_git(")))
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    holders.sort();

    // Control FIRST: a scan that matched nothing would pass this vacuously.
    assert!(
        !holders.is_empty(),
        "the scan found no `run_git` anywhere in src/; it did not run"
    );
    assert_eq!(
        holders,
        vec!["src/git.rs".to_owned(), "src/transport.rs".to_owned()],
        "the git process leg gained (or lost) a product holder"
    );
}

/// `transport::run_ps` is the FIXED-PROGRAM `ps` leg of the one process door —
/// the watchdog's per-cycle process-table snapshot.
#[test]
fn run_ps_has_exactly_one_product_caller() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut holders: Vec<String> = rust_sources()
        .into_iter()
        .filter(|p| p.starts_with(root.join("src")))
        .filter(|p| fs::read_to_string(p).is_ok_and(|text| text.contains("run_ps(")))
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    holders.sort();

    // Control FIRST: a scan that matched nothing would pass this vacuously.
    assert!(
        !holders.is_empty(),
        "the scan found no `run_ps` anywhere in src/; it did not run"
    );
    assert_eq!(
        holders,
        vec!["src/procs.rs".to_owned(), "src/transport.rs".to_owned()],
        "the ps process leg gained (or lost) a product holder"
    );
}

/// reviewer4's round-6 bypasses of this guard, from their tree, byte for byte.
const CFG_ATTR_EXEMPTION: &str = "#[cfg_attr(all(), allow(clippy::disallowed_types))]";

/// The second.
const EXPECT_EXEMPTION: &str =
    "#[expect(clippy::disallowed_types, reason = \"review exemption red proof\")]";

/// How many times `text` relaxes the disallowed-types lint.
fn lint_relaxations(text: &str) -> usize {
    // Assembled from halves so this file's own source does not contain the
    // needles.
    let relaxations = [
        concat!("allow(clippy::", "disallowed_types"),
        concat!("expect(clippy::", "disallowed_types"),
    ];
    let code = strip_literals(&strip_comments(text));
    let dense: String = code.split_whitespace().collect();
    relaxations
        .iter()
        .map(|needle| dense.matches(needle).count())
        .sum()
}

#[test]
fn the_exemption_counter_sees_the_forms_that_beat_its_last_version() {
    // Control first, as always: a counter that could never answer 1 would report
    // every door as absent.
    assert_eq!(
        lint_relaxations("#[allow(clippy::disallowed_types)] fn door() {}"),
        1,
        "the counter cannot see a plain allow"
    );

    for (shape, exemption) in [
        ("cfg_attr", CFG_ATTR_EXEMPTION),
        ("expect", EXPECT_EXEMPTION),
    ] {
        assert_eq!(
            lint_relaxations(&format!("{exemption}\nfn door() {{}}")),
            1,
            "{shape}: the exemption that beat the last version is still invisible"
        );
    }

    // And prose is still not a door.
    assert_eq!(
        lint_relaxations("// a comment mentioning #[allow(clippy::disallowed_types)]\nfn f() {}"),
        0,
        "a mention in a comment counted as a door"
    );
    assert_eq!(
        lint_relaxations("//! residual 4: a second #[expect(clippy::disallowed_types)] reopens it"),
        0,
        "a mention in module docs counted as a door"
    );
}

#[test]
fn the_lint_relaxations_this_counter_can_see_are_the_expected_ones() {
    // The capability boundary is `clippy.toml`'s `disallowed-types`, which
    // resolves TYPES: UFCS, `as` aliases and re-imports are all the same type to
    // it. That is what makes it close the CLASS, where a filter over method
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut inventory = Vec::new();
    for file in rust_sources() {
        let text =
            fs::read_to_string(&file).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
        let count = lint_relaxations(&text);
        if count > 0 {
            let named = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .into_owned();
            inventory.push((named, count));
        }
    }

    // Ten relaxations this counter can see, each for a different job: the
    // PRODUCT's three — `src/transport.rs`, because a tmux multiplexer that
    // cannot run tmux answers `unknown` about everything, `src/run.rs`, the
    assert_eq!(
        inventory,
        vec![
            ("src/install.rs".to_owned(), 1),
            ("src/run.rs".to_owned(), 1),
            ("src/transport.rs".to_owned(), 1),
            ("src/upgrade.rs".to_owned(), 1),
            ("tests/it/cli.rs".to_owned(), 5),
            ("tests/it/doors.rs".to_owned(), 1),
            ("tests/it/install.rs".to_owned(), 3),
            ("tests/it/parity.rs".to_owned(), 1),
            ("tests/it/shape.rs".to_owned(), 2),
        ],
        "the enumerated lint relaxations changed"
    );

    // And the harness's one door is the pinned one, not somewhere else in the file.
    let code = strip_literals(&strip_comments(&harness_source()));
    let wiring = body_of(&code, "mod raw");
    let token = concat!("clippy::", "disallowed_types");
    let at = code
        .find(token)
        .unwrap_or_else(|| panic!("the harness's door lost its `{token}` allow"));
    assert!(
        wiring.contains(&at),
        "the harness's door moved out of `mod raw`: {}",
        squashed(line_at(&code, at))
    );
}

#[test]
fn a_child_process_is_run_in_exactly_one_place_and_is_wrapped_there() {
    // DEFENCE IN DEPTH, and demoted deliberately. This filters three method
    // SPELLINGS, and a review walked past exactly that by writing
    // `std::process::Command::output(&mut command)` — semantically identical,
    let code = strip_literals(&strip_comments(&harness_source()));
    let wiring = body_of(&code, "mod raw");

    let mut sites = Vec::new();
    for spelling in [".output()", ".status()", ".spawn("] {
        let mut from = 0;
        while let Some(found) = code[from..].find(spelling) {
            let at = from + found;
            from = at + spelling.len();
            sites.push((spelling, at));
        }
    }

    assert_eq!(
        sites.len(),
        1,
        "a child is run in {} places in this harness; the wrapper only concentrates \
         the risk if it is the harness's only spawner: {:?}",
        sites.len(),
        sites.iter().map(|(how, _)| *how).collect::<Vec<&str>>()
    );
    assert!(
        wiring.contains(&sites[0].1),
        "the one call that runs a child is outside `mod raw`: {} — which is where \
         {RAW_STATUS_JUDGEMENT:?} put its own",
        squashed(line_at(&code, sites[0].1))
    );
}

/// The harness's own source — the file this guard protects.
fn harness_source() -> String {
    // `expect` is relaxed only inside `#[test]` bodies (clippy.toml); these are
    // helpers beside them, so they panic explicitly.
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/parity.rs"))
        .unwrap_or_else(|err| panic!("the harness source is readable: {err}"))
}

/// `source` with line and block comments removed, string literals intact.
fn strip_comments(source: &str) -> String {
    let src: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut index = 0;
    while index < src.len() {
        let current = src[index];
        let next = src.get(index + 1).copied();
        if current == '/' && next == Some('/') {
            while index < src.len() && src[index] != '\n' {
                index += 1;
            }
        } else if current == '/' && next == Some('*') {
            index += 2;
            while index < src.len() && !(src[index] == '*' && src.get(index + 1) == Some(&'/')) {
                index += 1;
            }
            index = src.len().min(index + 2);
            out.push(' ');
        } else if current == '"'
            || (current == 'r' && matches!(next, Some('"' | '#')))
            || (current == '\'' && is_char_literal(&src, index))
        {
            // A literal is copied through whole: `//` inside one is not a
            // comment, and this pass is only about the comments.
            let end = literal_end(&src, index);
            out.extend(&src[index..end]);
            index = end;
        } else {
            out.push(current);
            index += 1;
        }
    }
    out
}

/// `code` with the CONTENTS of string and char literals blanked out.
fn strip_literals(code: &str) -> String {
    let src: Vec<char> = code.chars().collect();
    let mut out = String::with_capacity(code.len());
    let mut index = 0;
    while index < src.len() {
        let current = src[index];
        let next = src.get(index + 1).copied();
        if current == '"'
            || (current == 'r' && matches!(next, Some('"' | '#')))
            || (current == '\'' && is_char_literal(&src, index))
        {
            let end = literal_end(&src, index);
            for blanked in &src[index..end] {
                // One space per BYTE, so a non-ASCII literal cannot shift an
                // offset the other view is about to be indexed by.
                for _ in 0..blanked.len_utf8() {
                    out.push(' ');
                }
            }
            index = end;
        } else {
            out.push(current);
            index += 1;
        }
    }
    out
}

/// The half-open byte range of the brace-delimited body that follows `header`.
fn body_of(code: &str, header: &str) -> Range<usize> {
    let sites = code.matches(header).count();
    assert_eq!(
        sites, 1,
        "`{header}` names {sites} sites in the harness; the exemption is one place, not a name"
    );
    let start = code
        .find(header)
        .unwrap_or_else(|| panic!("`{header}` is a site of this harness"));
    let open = start
        + code[start..]
            .find('{')
            .unwrap_or_else(|| panic!("`{header}` has a body"));
    let mut depth = 0usize;
    for (offset, byte) in code[open..].bytes().enumerate() {
        if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return open..open + offset + 1;
            }
        }
    }
    panic!("`{header}`'s body never closes");
}

/// The line `at` falls on, so a violation can be read rather than counted.
fn line_at(code: &str, at: usize) -> &str {
    let start = code[..at].rfind('\n').map_or(0, |index| index + 1);
    let end = code[at..].find('\n').map_or(code.len(), |index| at + index);
    &code[start..end]
}

/// Whitespace-collapsed and truncated, so a violation reads on one line.
fn squashed(chunk: &str) -> String {
    let text: String = chunk.split_whitespace().collect::<Vec<&str>>().join(" ");
    text.chars().take(96).collect()
}

/// Whether the `'` at `index` opens a char literal rather than a lifetime.
fn is_char_literal(src: &[char], index: usize) -> bool {
    src.get(index + 1) == Some(&'\\') || src.get(index + 2) == Some(&'\'')
}

/// The index just past the literal starting at `index`.
fn literal_end(src: &[char], index: usize) -> usize {
    let mut cursor = index;
    if src[cursor] == 'r' {
        cursor += 1;
        let mut hashes = 0;
        while src.get(cursor) == Some(&'#') {
            hashes += 1;
            cursor += 1;
        }
        if src.get(cursor) != Some(&'"') {
            return index + 1;
        }
        cursor += 1;
        while cursor < src.len() {
            if src[cursor] == '"' && (1..=hashes).all(|step| src.get(cursor + step) == Some(&'#')) {
                return cursor + hashes + 1;
            }
            cursor += 1;
        }
        return src.len();
    }
    let quote = src[cursor];
    cursor += 1;
    while cursor < src.len() {
        if src[cursor] == '\\' {
            cursor += 2;
            continue;
        }
        if src[cursor] == quote {
            return cursor + 1;
        }
        cursor += 1;
    }
    src.len()
}
