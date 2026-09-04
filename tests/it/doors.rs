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
///
/// It judged the RAW `std::process::Output` inside the capture site, before the
/// bytes ever became an opaque capture. No name rule could have caught it, and
/// it is kept here because of the line it OPENS with — `let output =
/// command.output()?;` — which is what
/// [`a_child_process_is_run_in_exactly_one_place_and_is_wrapped_there`] exists
/// to make unbuildable. The harness it was written against is gone; the door it
/// went through is not.
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
///
/// # Why this replaces the counting as the claim
///
/// `--force-warn` cannot be overridden by ANY relaxation: a plain `allow`, a
/// GROUP `allow` such as `clippy::style` or `clippy::all`, a `cfg_attr` wrapping
/// either, an `expect`, or a crate-root `#![allow]`. That is what the flag was
/// stabilised for, and it is why this asks the compiler instead of the text.
///
/// The counter below it enumerates relaxation FORMS, and enumerations in this
/// slice have now been beaten four times — a field-name list, a method-name
/// list, an outer-attribute prefix, and finally `#[allow(clippy::style)]`, which
/// relaxes `disallowed_types` by naming a GROUP the lint belongs to and no lint
/// name at all. A textual guard cannot see that without enumerating the group
/// graph too, which is the fifth shape waiting to happen.
///
/// # What this still does not cover
///
/// `RUSTFLAGS` or `--cap-lints` applied from OUTSIDE the tree, and anything that
/// changes what this guard itself runs. That is a nameable class rather than
/// "any relaxation nobody enumerated", and it is residual 4 in `parity.rs`.
fn command_sites_reported_by_clippy() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());

    // Its own target dir: the outer test run holds the normal one, and a guard
    // that blocks on someone else's lock is a guard that times out.
    // `--force-warn` is passed to the driver, so a cached result was produced
    // under the same flag and replays its diagnostics (measured — a run that
    // silently reported nothing because cargo considered the crate fresh would
    // be the vacuous-gate failure this whole file exists to avoid).
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

    // Non-vacuity FIRST. If the probe reports nothing, the interesting question
    // is not whether the doors moved — it is whether the probe ran at all.
    assert!(
        !sites.is_empty(),
        "the force-warn probe reported no `Command` anywhere; it did not run, and \
         a guard that scans nothing passes forever"
    );

    // Asked of the compiler, so no `allow` of any shape can hide a site from it:
    // these ARE the places this crate can start a child process.
    //
    // THREE product entries, and each is stated where it is used.
    // `src/transport.rs` runs tmux: ae cannot answer SC-017k/SC-017l without
    // it, and before it existed every session ae listed read `unknown` by
    // construction. `src/run.rs` is the pane's own `exec` — it BECOMES the
    // tool rather than starting a child, which is the fact
    // `pane_current_command` rests on, and it arrived with slice Z2 when the
    // generated `launch.<slot>.sh` that used to hold that `exec` was deleted.
    // `src/install.rs` and `src/upgrade.rs` are slice Z4's, when the
    // installer's logic moved out of bash. `install.rs` runs the
    // DIGEST-VERIFIED bundle core ONCE, with one fixed argument, to ask which
    // version it is — the version directory is named for that answer, and the
    // install gate compares the two on every later invocation, so asking here
    // turns a mis-named publish into an install-time refusal instead of a
    // bricked install. `upgrade.rs` no longer `exec`s the sibling installer at
    // all; its door is `tar`, listing and then unpacking the bundle it
    // downloaded, and the listing pass is what makes running it safe.
    // All four are listed rather than exempted because the value of this guard
    // is that adding a door is a line in a review, not a diff nobody read.
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

/// `transport::run_git` is the FIXED-PROGRAM git leg of the one process door: it
/// chooses the binary (`git`) so a caller only chooses arguments. The PRIMARY
/// boundary is a TYPE: `run_git` takes a `git::GitArgv` whose inner vector is
/// private to `src/git.rs`, so an alias-import (`use … run_git as invoke_git;`)
/// is inert — it cannot mint the argv it would need. This guard is defence in
/// depth beside that seal: within `src/`, the INVOCATION form `run_git(` appears
/// in exactly two files — `transport.rs`, which DEFINES it, and `git.rs`, its
/// one product caller. A third file gaining a call is a line in a review, not a
/// diff nobody read. The token is `run_git(` (with the open paren) on purpose: a
/// doc-link that merely NAMES the function — good docs — carries no paren and is
/// not a caller. (Test code is out of scope: a test may drive the door; product
/// code may not widen who holds it.)
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
/// the watchdog's per-cycle process-table snapshot. Same seal as [`run_git`]: it
/// takes a `procs::PsArgv` whose inner vector is private to `src/procs.rs`, and
/// that argv carries NO caller input at all (the snapshot spelling is a
/// constant), so there is nothing to inject even in principle. This guard is
/// defence in depth beside that seal: within `src/`, the INVOCATION form
/// `run_ps(` appears in exactly two files — `transport.rs`, which DEFINES it,
/// and `procs.rs`, its one product caller (`snapshot`). A third file gaining a
/// call is a line in a review, not a diff nobody read.
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
///
/// Both were rustfmt-clean, clippy-clean and left all three boundary tests
/// green with a USED UFCS back door underneath. They are here because the guard
/// they beat was looking for the outer attribute `#[allow(` rather than for the
/// LINT RELAXATION, and an enumeration that has been beaten twice should carry
/// the two it missed.
const CFG_ATTR_EXEMPTION: &str = "#[cfg_attr(all(), allow(clippy::disallowed_types))]";

/// The second. See [`CFG_ATTR_EXEMPTION`].
const EXPECT_EXEMPTION: &str =
    "#[expect(clippy::disallowed_types, reason = \"review exemption red proof\")]";

/// How many times `text` relaxes the disallowed-types lint.
///
/// Counts the RELAXATION, not the attribute that carries it: `allow(...)` and
/// `expect(...)` are found wherever they are nested — inside `cfg_attr`, inside
/// an inner `#![...]`, inside anything else that has not been invented yet —
/// because the outer wrapper is precisely what the previous version enumerated
/// and precisely what was laundered past.
///
/// Comments are stripped first, and string literals blanked: naming the
/// attribute in prose — as `parity.rs`'s residual list does — or quoting one as
/// test data — as the two constants above do — is documentation, not a door. An
/// attribute cannot be inside a string literal and still be an attribute. This
/// guard caught both of those on its first run after each change, which is the
/// right failure to have had twice.
///
/// **This is an enumeration, it closes only what it enumerates, and it has been
/// PROVEN incomplete.** `#[allow(clippy::style)]` relaxes `disallowed_types` by
/// naming a GROUP the lint belongs to — no lint name appears at all, so nothing
/// here sees it, and a review used exactly that with a working third `Command`
/// site in `src/`, green in every lane including this counter.
///
/// It is kept as defence in depth because it reads well in a failure and costs
/// nothing. The CLAIM is
/// [`the_capability_boundary_holds_against_any_lint_relaxation`], which asks
/// clippy under `--force-warn` and cannot be relaxed by any attribute at all.
fn lint_relaxations(text: &str) -> usize {
    // Assembled from halves so this file's own source does not contain the
    // needles. A guard that counts a token must not itself be a place that
    // token can hide, and excluding its own file would be exactly that.
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
    // names closed one spelling of it.
    //
    // What a deny cannot stop is a second RELAXATION of the lint. `forbid`
    // would, and would also block the door itself — so this counts them instead.
    //
    // DEMOTED, and provably so. This counts relaxation FORMS, and a review beat
    // it with `#[allow(clippy::style)]` — a GROUP allow, which relaxes
    // `disallowed_types` while naming no lint at all — carrying a working third
    // `Command` site. It also does not see a crate-root `#![allow]` in a new
    // file, or a feature-gated `cfg_attr`.
    //
    // The CLAIM is now
    // `the_capability_boundary_holds_against_any_lint_relaxation`, which asks
    // the compiler under `--force-warn` and no attribute can override. This
    // remains only as defence in depth: a changed inventory is worth seeing
    // even when the semantic guard would also catch it, and it names the file.
    //
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
    // pane's own `exec` of its tool, and `src/upgrade.rs`, the `exec` that
    // hands the terminal to the immutable sibling installer; the parity
    // harness's door,
    // which must never judge a lane; the black-box door, which drives the
    // PRODUCT binary and where asserting on what it printed is the whole point
    // (`cli::ae` is private to its module, so the harness cannot reach a child
    // through it); the black-box FIFO fixture beside it (`cli::mkfifo` — safe
    // std cannot make the one special file that blocks an ungated open, and
    // the tests that prove the `-f` gates need exactly that file); the
    // by-name door beside it (`cli::helper_by_name` — a helper's identity IS
    // `argv[0]`, so proving the bare-name refusal needs a process started AS
    // the name, which no path spelling produces); the git
    // fixture builder beside those (`cli::git_in` — the preview's git-facts
    // tests need REAL repos, and only `git` builds a real repo); the generated
    // session-helper runner beside those too (`cli::helper` — a launch writes
    // shims a pane execs BY PATH, so proving one works means running the file
    // rather than the function behind it); and this file's own, which has to
    // run clippy in order to ask clippy anything.
    //
    // A TENTH DOOR arrived with slice Z3 and is the installed shape's:
    // `tests/it/shape.rs` runs a COPY of the product binary planted in a
    // fixture version directory, twice, because the fact under test is
    // `current_exe()` — where the binary SITS — and no library call can produce
    // it. `cli::ae()` cannot be used: it names the built binary under `target/`,
    // which is a CHECKOUT by construction and so the wrong arm entirely.
    //
    // TWO MORE ARRIVED WITH SLICE Z4, when the installer's logic moved out of
    // bash. `src/install.rs` runs the DIGEST-VERIFIED bundle core once to ask
    // which version it is, because the version directory is named for that
    // answer and the install gate compares the two on every later invocation.
    // `src/upgrade.rs`'s door changed job rather than arriving: it used to
    // `exec` the sibling installer, and now runs `tar` to list and unpack the
    // bundle it downloaded — a deliberate non-dependency, gated by the listing
    // pass that proves every entry before anything is extracted.
    // `tests/it/install.rs` is the black-box door for both: an install is what
    // a real process does to a real `$HOME`.
    //
    // A THIRD SITE IN THAT FILE is the bash bootstrap's: `install.rs`'s
    // `command()` runs `bash` over the repository's own `install`, and `tar` to
    // pack the fixture bundle its shimmed `curl` serves. Everything the
    // bootstrap owns happens BEFORE there is a core — resolve the platform,
    // prove the archive against the release manifest, extract, exec — so there
    // is no library call that could stand in for the process.
    //
    // A further entry is red. A relaxation this counter CANNOT see is not —
    // that is what the semantic guard above is for.
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
    // matching none of them. Adding `::output` to the list would have closed
    // that spelling, not the class.
    //
    // What closes the class is the denied TYPE in `clippy.toml`, which resolves
    // paths. This test survives underneath it because a second call site inside
    // the harness is worth seeing even when it is legitimately allowed, and
    // because it reads well in a failure. It is not the boundary.
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
///
/// The exclusion is the point: the harness's own docs have to be able to SAY
/// "no `#[test]` lives here" without the guard reading its own prose as the
/// violation. The first-pass version of this test failed on exactly that.
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
///
/// `join("stdout")` names an artifact file; it is not the harness reading one.
/// Blanking the contents keeps that distinction — and keeps a `;` or a `"`
/// inside a literal from desynchronising the statement split below.
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
///
/// Brace matching is exact here, and it is exact because of WHERE it runs: on
/// source whose comments are gone and whose literal contents have been blanked.
/// A `{` in a doc comment, or the `{}` of a format string, cannot desynchronise
/// it, because by this point neither still exists.
///
/// `header` must name exactly one site. A second one would be a second place
/// evidence is allowed to live, which is the thing this whole rule exists to
/// deny — so it fails loudly rather than picking the first.
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
