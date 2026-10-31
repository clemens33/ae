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
//! The suite has ONE child-process door, [`super::parity`]'s `Invocation` +
//! `capture::raw`, and the last test below is about that door.

#![allow(
    clippy::disallowed_methods,
    reason = "these read the crate's own source and config; the capability boundary is \
              about what PRODUCT code may reach"
)]

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::Path;

use ae::json::{self, Value};

use crate::support::source_view;

/// One of reviewer4's round-4 injections against the parity harness, byte for
/// byte.
const RAW_STATUS_JUDGEMENT: &str = "        let output = command.output()?;
        if std::env::var_os(\"AE_PARITY_STATUS_JUDGE\").is_some() && !output.status.success() {
            return Err(io::Error::other(\"lane status differed\"));
        }
";

/// The `src/` files whose WHOLE text (tests included) holds `needle`, sorted.
/// The text is the shared loader's, so a file that cannot be read fails the
/// test by name instead of dropping out of the scan.
fn src_holders(needle: &str) -> Vec<String> {
    let mut found: Vec<String> = source_view::tree()
        .iter()
        .filter(|(path, text)| path.starts_with("src/") && text.contains(needle))
        .map(|(path, _)| path.clone())
        .collect();
    found.sort();
    found
}

/// One file of the shared tree, whole.
fn tree_text(path: &str) -> &'static str {
    source_view::tree()
        .iter()
        .find(|(name, _)| name == path)
        .map_or_else(
            || panic!("{path} was not loaded"),
            |(_, text)| text.as_str(),
        )
}

/// `text` with comments, then literals, blanked to spaces: the offsets stay the
/// source's own and nothing a body search reads is prose or a string. A lexical
/// fault fails by `file:line`.
fn structural(file: &str, text: &str) -> String {
    source_view::without_comments(file, text)
        .and_then(|code| source_view::blank_literals(file, &code))
        .unwrap_or_else(|failure| panic!("{failure}"))
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
        .env("CARGO_INCREMENTAL", "0")
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
            "tests/it/migrate.rs".to_owned(),
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
    let holders = src_holders("run_git(");

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
    let holders = src_holders("run_ps(");

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

/// The board's streaming door lives at exactly ONE product site: the
/// transcript open in `src/board.rs`, reused by every harness reader. A second
/// opener anywhere else is a review, not a diff.
#[test]
fn the_board_streaming_door_has_exactly_one_product_site() {
    let holders = src_holders("stream_transcript(");

    // Control FIRST: a scan that matched nothing would pass this vacuously.
    assert!(
        !holders.is_empty(),
        "the scan found no `stream_transcript` anywhere in src/; it did not run"
    );
    assert_eq!(
        holders,
        vec!["src/board.rs".to_owned()],
        "the board streaming door gained (or lost) a product site"
    );

    // And the site opens exactly once: a second `File::open` in the same file
    // is the same bypass one file over.
    let board = tree_text("src/board.rs");
    assert_eq!(
        board.matches("File::open").count(),
        1,
        "src/board.rs opens more than once"
    );
}

/// `transport::run_sysctl` is the FIXED-PROGRAM boot-time leg of the one
/// process door — and it takes no arguments at all, because there is exactly
/// one question ae asks `sysctl`.
#[test]
fn run_sysctl_has_exactly_one_product_caller() {
    let holders = src_holders("run_sysctl(");

    // Control FIRST: a scan that matched nothing would pass this vacuously.
    assert!(
        !holders.is_empty(),
        "the scan found no `run_sysctl` anywhere in src/; it did not run"
    );
    assert_eq!(
        holders,
        vec!["src/doors.rs".to_owned(), "src/transport.rs".to_owned()],
        "the sysctl process leg gained (or lost) a product holder"
    );
}

/// `transport::run_agy_quota`, the network-reaching agy leg, runs on demand only,
/// and its child's streams are never pipes that could block ae past a deadline.
#[test]
fn run_agy_quota_has_exactly_one_product_caller_and_no_pipe() {
    let holders = src_holders("run_agy_quota");
    // A scan that found nothing fails here too: the expected list is not empty.
    assert_eq!(
        holders,
        ["src/quota.rs", "src/transport.rs"],
        "the agy process leg gained (or lost) a product holder"
    );
    let code = structural("src/transport.rs", tree_text("src/transport.rs"));
    let body = &code[body_of(&code, "fn spawn_until")];
    assert_eq!(
        body.matches("Stdio::null()").count(),
        2,
        "stdin and stderr are no longer both closed to the agy child"
    );
    assert!(
        !body.contains("piped") && !body.contains("inherit"),
        "the agy leg gained a pipe or an inherited stream"
    );
}

/// `transport::run_clipboard_image` is the FIXED-PROGRAM clipboard leg of
/// the one process door: it chooses the binary and every argument.
#[test]
fn run_clipboard_image_has_exactly_one_product_caller() {
    let holders = src_holders("run_clipboard_image(");
    // Control FIRST: a scan that matched nothing would pass this vacuously.
    assert!(!holders.is_empty(), "empty scan did not run");
    assert_eq!(
        holders,
        ["src/clipboard.rs", "src/transport.rs"],
        "the clipboard process leg gained (or lost) a product holder"
    );
}
/// The opencode leg captures through a SCRATCH FILE, never a pipe: the real
/// `opencode export` exits before its stdout pipe drains (measured on 1.18.31,
/// 2026-09-18: 131072 of 3949726 bytes arrived through a pipe, the whole
/// document through a file), so a pipe capture truncates the JSON document and
/// the board covers a whole conversation as unreadable. The guard is lexical on
/// purpose: no shell fixture can reproduce a runtime that abandons queued
/// output at exit, so the wiring itself is what is pinned.
#[test]
fn the_opencode_leg_captures_through_a_file_never_a_pipe() {
    let code = structural("src/transport.rs", tree_text("src/transport.rs"));
    let body = &code[body_of(&code, "fn run_opencode")];
    assert!(
        body.contains("CapturedToFile"),
        "the opencode leg lost its file capture"
    );
    assert!(
        !body.contains("Streams::Captured,"),
        "the opencode leg regressed to a pipe capture; opencode exits before its pipe drains"
    );
}

/// R3/R4: the BOOT-TIME proof is reachable from exactly the named operations,
/// through the one worker end and rename share — and from nowhere else.
///
/// `compact` and `stop` are destructive and irreversible, and they keep
/// `verify_session_absent`'s strict proof — a session is gone because the
/// server said so, never because a socket is missing and the arithmetic worked
/// out. `end` crosses the boot proof ONLY through
/// `lifecycle::end::boot_proved_stopped`, and only behind `--assume-stopped`;
/// `rename` crosses the same gate (it deletes no state, so it needs no
/// acknowledgement). This is the guard that a later refactor cannot quietly
/// wire the weaker evidence somewhere else.
#[test]
fn the_boot_time_proof_is_reachable_from_exactly_its_named_operations() {
    let holders = src_holders;

    // Control first, both ways: a needle that matches nothing passes forever,
    // and one that matches everything says nothing.
    for needle in [
        "classify_absence(",
        "probe_absence(",
        "boot_time(",
        "boot_proved_stopped(",
    ] {
        assert!(
            !holders(needle).is_empty(),
            "the scan found no `{needle}` anywhere in src/; it did not run"
        );
    }

    assert_eq!(
        holders("classify_absence("),
        vec![
            // The listing, through its fleet discovery's `all_predate`.
            "src/inventory.rs".to_owned(),
            // The ONE boot-proof gate `end` and `rename` share.
            "src/lifecycle/end.rs".to_owned(),
            // The resume, at `resume_absence`.
            "src/session_launch.rs".to_owned(),
            // Its own definition and unit table.
            "src/tmux.rs".to_owned(),
        ],
        "the boot-time verdict gained (or lost) a caller"
    );
    assert_eq!(
        holders("probe_absence("),
        vec![
            // The shared gate.
            "src/lifecycle/end.rs".to_owned(),
            "src/session_launch.rs".to_owned(),
            "src/transport.rs".to_owned(),
        ],
        "the raw absence probe gained (or lost) a caller"
    );
    assert_eq!(
        holders("boot_time("),
        vec![
            // The doctor's own absence reading.
            "src/doctor.rs".to_owned(),
            // Its definition and its platform table.
            "src/doors.rs".to_owned(),
            // The fleet listing's discovery.
            "src/lib.rs".to_owned(),
            // The shared gate.
            "src/lifecycle/end.rs".to_owned(),
            "src/session_launch.rs".to_owned(),
        ],
        "the boot time gained (or lost) a reader"
    );
    assert_eq!(
        holders("boot_proved_stopped("),
        vec![
            // The one definition, called by `end`'s Positive arm.
            "src/lifecycle/end.rs".to_owned(),
            // The rename preflight and its retry.
            "src/rename.rs".to_owned(),
        ],
        "the shared boot-proof gate gained (or lost) a caller"
    );

    // The holder SET above says WHICH files may reach the proof; it cannot see
    // a second, ungated call inside one of them. `end.rs` and `rename.rs` are
    // EXPECTED to hold these primitives, so the COUNT is the guard: a later
    // refactor that adds another `classify_absence(` / `probe_absence(` /
    // `boot_time(` anywhere but the one shared composition goes RED here.
    let count_in = |file: &str, needle: &str| tree_text(file).matches(needle).count();
    for needle in ["classify_absence(", "probe_absence(", "boot_time("] {
        assert_eq!(
            count_in("src/lifecycle/end.rs", needle),
            1,
            "end.rs must compose `{needle}` exactly once, inside boot_proved_stopped"
        );
    }
    assert_eq!(
        count_in("src/lifecycle/end.rs", "boot_proved_stopped("),
        2,
        "end.rs must hold the shared gate as one definition and exactly one call (the Positive arm)"
    );
    assert_eq!(
        count_in("src/rename.rs", "boot_proved_stopped("),
        2,
        "rename.rs must call the shared gate from the preflight and the retry, and nowhere else"
    );

    // And the two verbs that still refuse on the strict proof alone.
    for (file, operation) in [("src/compact.rs", "compact"), ("src/lifecycle.rs", "stop")] {
        let code = tree_text(file);
        assert!(
            !code.contains("classify_absence(")
                && !code.contains("probe_absence(")
                && !code.contains("boot_time(")
                && !code.contains("boot_proved_stopped("),
            "{operation} ({file}) reached the boot-time proof; it must keep the strict one"
        );
    }
}

/// reviewer4's round-6 bypasses of this guard, from their tree, byte for byte.
const CFG_ATTR_EXEMPTION: &str = "#[cfg_attr(all(), allow(clippy::disallowed_types))]";

/// The second.
const EXPECT_EXEMPTION: &str =
    "#[expect(clippy::disallowed_types, reason = \"review exemption red proof\")]";

/// How many times `text` (the source of `file`) relaxes the disallowed-types
/// lint.
fn lint_relaxations(file: &str, text: &str) -> usize {
    // Assembled from halves so this file's own source does not contain the
    // needles.
    let relaxations = [
        concat!("allow(clippy::", "disallowed_types"),
        concat!("expect(clippy::", "disallowed_types"),
    ];
    let code = structural(file, text);
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
        lint_relaxations("fixture", "#[allow(clippy::disallowed_types)] fn door() {}"),
        1,
        "the counter cannot see a plain allow"
    );

    for (shape, exemption) in [
        ("cfg_attr", CFG_ATTR_EXEMPTION),
        ("expect", EXPECT_EXEMPTION),
    ] {
        assert_eq!(
            lint_relaxations("fixture", &format!("{exemption}\nfn door() {{}}")),
            1,
            "{shape}: the exemption that beat the last version is still invisible"
        );
    }

    // And prose is still not a door.
    assert_eq!(
        lint_relaxations(
            "fixture",
            "// a comment mentioning #[allow(clippy::disallowed_types)]\nfn f() {}"
        ),
        0,
        "a mention in a comment counted as a door"
    );
    assert_eq!(
        lint_relaxations(
            "fixture",
            "//! residual 4: a second #[expect(clippy::disallowed_types)] reopens it"
        ),
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
    let mut inventory = Vec::new();
    for (file, text) in source_view::tree() {
        let count = lint_relaxations(file, text);
        if count > 0 {
            inventory.push((file.clone(), count));
        }
    }

    // Twenty relaxations over ten files, each for a job no library call can
    // do. The PRODUCT's five: `src/transport.rs`, because a tmux multiplexer
    // that cannot run tmux answers `unknown` about everything; `src/run.rs`,
    // the pane's own `exec` of its tool; `src/upgrade.rs` TWICE — `tar`, to
    // list and unpack the bundle it downloaded, and then the downloaded core's
    // own `_install`, because the migration chain a publish runs belongs to the
    // version being INSTALLED and not to the one doing the installing; and
    // `src/install.rs`, which runs the digest-verified bundle core once to ask
    // which version it is, because the directory is named for that answer.
    //
    // The suite's fifteen: the parity harness's one, which must never judge a
    // lane; `cli.rs`'s six, which drive the PRODUCT binary (`ae`), make the
    // one special file safe std cannot (`mkfifo`, and an ungated open on it is
    // what the `-f` gates exist to refuse), start a helper AS its own name
    // (`helper_by_name`, since a helper's identity IS `argv[0]`), build a real
    // repo (`git_in`), run a written shim BY PATH (`helper`), and hold an
    // attached tmux client a watched-pane test can be refused by
    // (`tmux_attached_client`, since only a real process is listed);
    // `install.rs`'s four: an install being what a real process does to a real
    // `$HOME`, a SECOND publisher started while the first still runs, an
    // `upgrade` refusal being a real process's exit status, and the bootstrap,
    // which is `bash` over the repository's own `install` with `tar` packing
    // the fixture bundle its shimmed `curl` serves; `shape.rs`'s two,
    // which plant a COPY of the binary in a fixture version directory because
    // the fact under test is where the binary SITS; `migrate.rs`'s one, which
    // publishes into a real `$HOME` to prove that a real install repoints real
    // sessions; and this file's own, which has to run clippy in order to ask
    // clippy anything.
    //
    // A further entry is red. A relaxation this counter CANNOT see is not —
    // that is what the semantic guard above is for.
    assert_eq!(
        inventory,
        vec![
            ("src/install.rs".to_owned(), 1),
            ("src/run.rs".to_owned(), 1),
            ("src/transport.rs".to_owned(), 1),
            ("src/upgrade.rs".to_owned(), 2),
            ("tests/it/cli.rs".to_owned(), 6),
            ("tests/it/doors.rs".to_owned(), 1),
            ("tests/it/install.rs".to_owned(), 4),
            ("tests/it/migrate.rs".to_owned(), 1),
            ("tests/it/parity.rs".to_owned(), 1),
            ("tests/it/shape.rs".to_owned(), 2),
        ],
        "the enumerated lint relaxations changed"
    );

    // And the harness's one door is the pinned one, not somewhere else in the file.
    let code = harness_code();
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
    let code = harness_code();
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

/// The harness's own source — the file this guard protects — structural.
fn harness_code() -> String {
    structural("tests/it/parity.rs", tree_text("tests/it/parity.rs"))
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

/// Every `src/**.rs` production half, from the shared source view
/// ([`source_view::product`]): comments and every `#[cfg(test)]` item blanked
/// in place, files registered by test code alone left out, a file the view
/// cannot classify a failure naming `file:line`. Nothing is cut at a first
/// gate, so production after a test module is scanned too.
fn product_halves() -> Vec<(String, String)> {
    source_view::product()
        .files
        .iter()
        .map(|file| (file.path.clone(), file.text.clone()))
        .collect()
}

#[test]
fn the_door_scan_reaches_app_registrations_and_leaves_test_only_files_out() {
    let halves = product_halves();
    let last = crate::phase2::last_ungated_app_registration();
    assert!(
        halves
            .iter()
            .any(|(name, code)| name == "src/app.rs" && code.lines().any(|line| line == last)),
        "the scan must reach {last:?} past app.rs's first gate"
    );
    assert!(
        halves
            .iter()
            .all(|(name, _)| name != "src/app/fleet_spec.rs"),
        "a registered test-only spec file is not product"
    );
    assert!(
        source_view::product()
            .test_only
            .iter()
            .any(|name| name == "src/app/fleet_spec.rs"),
        "and the view names it as test-only, rather than not finding it at all"
    );
}

/// The bodies of production functions, with braces inside literals ignored.
/// This is enough structure for the mutation guard below to associate a
/// direct filesystem operation with the session container that it targets.
fn function_bodies(code: &str) -> Vec<&str> {
    let blanked = structural("product half", code);
    let mut bodies = Vec::new();
    for (start, _) in blanked.match_indices("fn ") {
        let Some(open_offset) = blanked[start..].find('{') else {
            continue;
        };
        let open = start + open_offset;
        let mut depth = 0usize;
        for (offset, byte) in blanked[open..].bytes().enumerate() {
            if byte == b'{' {
                depth += 1;
            } else if byte == b'}' {
                depth -= 1;
                if depth == 0 {
                    bodies.push(&code[start..=open + offset]);
                    break;
                }
            }
        }
    }
    bodies
}

/// Find a direct mutation of a live event or memo container in one function.
/// This is a conservative lexical tripwire over known path/mutator spellings,
/// not a data-flow proof: a co-occurrence in one function is enough to reject
/// a likely direct writer, while aliases and unrelated paths can escape it.
fn direct_session_mutation(code: &str) -> Option<&str> {
    let targets = [
        "events_path()",
        "memo_path()",
        "store::EVENTS",
        "store::MEMO",
    ];
    let mutators = [
        "fs::write(",
        "File::create(",
        "OpenOptions::new(",
        "fs::rename(",
    ];
    function_bodies(code).into_iter().find(|body| {
        targets.iter().any(|target| body.contains(target))
            && mutators.iter().any(|mutator| body.contains(mutator))
    })
}

/// `src/init.rs` owns ae's ONE exclusive publication — a temp under a nonce
/// only that call opened, fsynced, then hard-linked onto a name that must not
/// already exist — and every other product caller of it is NAMED here. The
/// caller list is ONE table: a file reaches the boundary only by appearing in
/// it with the exact spelling it must use, so the deny below and the per-caller
/// proof above can never drift apart into a silent exemption.
#[test]
fn init_owns_exclusive_publication_and_every_caller_of_it_is_named() {
    /// Each product file that may call the boundary, and the exact call it must
    /// make. The mode is part of the spelling: it is the published file's, and
    /// a caller that changed it would be publishing into someone else's store
    /// at the wrong permissions.
    const CALLERS: [(&str, &str, &str); 4] = [
        (
            "src/lib.rs",
            "crate::init::create_exclusive(path, contents.as_bytes(), 0o666)",
            "first-launch seeding must race through init's same exclusive writer and retain its umask-derived mode",
        ),
        // THE SECOND LEGITIMATE CALLER, and the reason it is not a second copy:
        // an account carry publishes a conversation file into a store ae does
        // not hold a lock on, so it needs exactly this operation — a temp under
        // a nonce that call opened, then a link that refuses an existing name —
        // and a duplicate of it would be a second no-clobber publication to get
        // wrong. Its mode is the store's, not the config's.
        (
            "src/carry.rs",
            "crate::init::create_exclusive(path, bytes, 0o600)",
            "the carry's publication must be init's exclusive writer at the store's own mode",
        ),
        // THE THIRD LEGITIMATE CALLER: a stopped rename publishes its identity
        // witness into the managed work root it is about to move (git mode and
        // full mode only — local mode moves nothing and writes no witness), so
        // it needs exactly this operation, and a duplicate would be a second
        // no-clobber publication to get wrong. 0o644 is the plain-file mode:
        // the witness is not secret (its UUID also sits in the intent).
        (
            "src/rename.rs",
            "crate::init::create_exclusive(&path, witness_bytes(nonce).as_bytes(), 0o644)",
            "the witness publication must be init's exclusive writer at plain-file mode",
        ),
        (
            "src/clipboard.rs",
            "crate::init::create_exclusive(&path, &bytes, 0o600)",
            "the clipboard image publication must be init's exclusive writer at the store's own mode",
        ),
    ];

    let halves = product_halves();
    let half = |wanted: &str| {
        halves
            .iter()
            .find(|(name, _)| name == wanted)
            .map_or_else(|| panic!("{wanted} was not scanned"), |(_, code)| code)
    };
    let init = half("src/init.rs");
    assert!(
        init.contains(".create_new(true)"),
        "init's staging writer must retain O_EXCL"
    );
    assert!(
        init.contains("file.write_all(bytes).and_then(|()| file.sync_all())")
            && init.contains("fs::hard_link(&temp, path)")
            && init.contains("let _ = fs::remove_file(&temp)"),
        "exclusive publication must fsync a temp, hard-link without clobbering, then unlink the temp"
    );
    assert!(
        init.contains("fs::rename(&temp, path)"),
        "forced config replacement must stay atomic rename"
    );
    for (name, call, why) in CALLERS {
        assert!(half(name).contains(call), "{why}");
    }
    for (name, code) in &halves {
        if name == "src/init.rs" || CALLERS.iter().any(|(caller, _, _)| caller == name) {
            continue;
        }
        assert!(
            !code.contains("create_exclusive("),
            "{name} gained a caller of init's exclusive-publication boundary"
        );
    }
}

/// DEBT, not design: production tool decisions outside `src/tool.rs` that this
/// guard first saw once its cut reached past `quota.rs:45`. A row is keyed by
/// file, needle and EXACT count, never a file skip: an extra branch fails, and
/// so does a removed branch whose row stays. Slice `toolrows` moves each
/// branch into an adapter row and deletes its row here.
const TOOL_BRANCH_DEBT: [(&str, &str, usize); 3] = [
    // `model_family` and its family helper: model-scoped quota windows are
    // Claude-only today (slice toolrows).
    ("src/quota.rs", "ToolKind::Claude", 2),
    // The agy quota leg: the spoken-groups filter and the home-inheriting
    // leg (slice toolrows).
    ("src/quota.rs", "ToolKind::Agy", 2),
    // Muse quota leg debt (slice toolrows), seven distinct decisions:
    // muse_home: select the opted-in scope's config home.
    // muse_legs: select scopes the Muse leg may speak for.
    // replace_muse: replace placeholders for the scopes that leg spoke for.
    // configured_scopes_with: attach unusable-config notes only to Muse scopes.
    // absorb_resolved (asks): admit the paid leg only when Muse is opted in.
    // absorb_resolved (identity): use the config home as the keychain account identity.
    // leg_hint: show a Muse hint only after its leg spoke.
    ("src/quota.rs", "ToolKind::Muse", 7),
];

/// Contiguous `ToolKind::UppercaseFirst` tokens, without an enum vocabulary
/// that could omit a current or future variant. Lowercase associated functions
/// are not variant decisions; the full token keeps variant prefixes distinct.
fn tool_variant_tokens(code: &str) -> impl Iterator<Item = &str> {
    code.match_indices("ToolKind::").filter_map(|(at, prefix)| {
        if code[..at]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            return None;
        }
        let rest = &code[at + prefix.len()..];
        if !rest.as_bytes().first().is_some_and(u8::is_ascii_uppercase) {
            return None;
        }
        let len = rest
            .find(|ch: char| !ch.is_alphanumeric() && ch != '_')
            .unwrap_or(rest.len());
        Some(&code[at..at + prefix.len() + len])
    })
}

#[test]
fn tool_variant_tokens_follow_spelling_not_the_current_enum() {
    assert_eq!(
        tool_variant_tokens(
            "ToolKind::Muse ToolKind::MuseExtra crate::tool::ToolKind::Zeta \
             ToolKind::from_binary_name ToolKind::from_known_binary_name \
             OtherToolKind::Muse ToolKind::"
        )
        .collect::<Vec<_>>(),
        ["ToolKind::Muse", "ToolKind::MuseExtra", "ToolKind::Zeta"]
    );
}

/// Lexical tripwire for direct, uppercase-first `ToolKind::Variant` spellings
/// in each production half (the view `source_view` owns: every `#[cfg(test)]`
/// item and comment blanked in place, files registered by test code alone left
/// out, production after a test module still scanned), less the exact-count
/// `TOOL_BRANCH_DEBT` rows. Aliases, glob-imported variants, indirect decisions
/// and strings are outside what this source-text check can prove; it does not
/// establish semantic ownership.
#[test]
fn per_tool_branches_live_only_in_the_adapter_rows() {
    let halves = product_halves();
    let adapter = halves
        .iter()
        .find(|(name, _)| name == "src/tool.rs")
        .map_or_else(
            || panic!("the adapter source was not scanned"),
            |(_, code)| code,
        );
    assert!(
        tool_variant_tokens(adapter).next().is_some(),
        "src/tool.rs has no ToolKind variant tokens in its production view; \
         verify the view and adapter spelling"
    );
    let needles: BTreeSet<_> = halves
        .iter()
        .flat_map(|(_, code)| tool_variant_tokens(code))
        // Retain debt needles even if their last source occurrence disappears.
        .chain(TOOL_BRANCH_DEBT.iter().map(|(_, needle, _)| *needle))
        .collect();
    for (file, _, _) in TOOL_BRANCH_DEBT {
        assert!(
            halves.iter().any(|(name, _)| name == file),
            "debt row names {file}, which was not scanned"
        );
    }
    for needle in needles {
        for (name, code) in &halves {
            if name == "src/tool.rs" {
                continue;
            }
            let owed = TOOL_BRANCH_DEBT
                .iter()
                .find(|(file, debt, _)| file == name && *debt == needle)
                .map_or(0, |row| row.2);
            assert_eq!(
                tool_variant_tokens(code)
                    .filter(|token| *token == needle)
                    .count(),
                owed,
                "{name} branches on {needle} other than its {owed} debt row(s); \
                 production tool decisions belong in src/tool.rs"
            );
        }
    }
}

#[test]
fn a_live_session_file_is_named_in_exactly_one_place() {
    // A second spelling of `events.jsonl` or `memo.tsv` is how a reader and a
    // writer end up on two different files, and a hand-appended `.lock` is how
    // one mutual exclusion becomes two. `src/store.rs` owns both names and
    // derives every lock from them, so a production line anywhere else that
    // writes one out is the drift this guard exists to catch. The literals are
    // unterminated on purpose: `"events.jsonl.lock"` is caught by the same
    // check as `"events.jsonl"`. Fixtures are not production — the scan is
    // shown each file with its `#[cfg(test)]` items blanked.
    for (name, code) in product_halves() {
        // `src/archive/store.rs` names an ARCHIVE's members. They coincide with
        // the live names today and are deliberately independent: an archive is
        // immutable, so renaming a live file must not rename what is published.
        if name == "src/store.rs" || name == "src/archive/store.rs" {
            continue;
        }
        // The file and its lock, exactly. A DERIVED sibling built from the
        // name (`session_launch`'s `events.jsonl.trim.<pid>` staging file) is
        // not a second spelling of either, and is not what drifts.
        for literal in [
            "\"events.jsonl\"",
            "\"events.jsonl.lock\"",
            "\"memo.tsv\"",
            "\"memo.tsv.lock\"",
            "\"meta.lock\"",
        ] {
            assert!(
                !code.contains(literal),
                "{name} spells {literal} itself; a live session file is named in src/store.rs"
            );
        }
        // `meta` is checked as a RAW string rather than in one path-building
        // spelling, because it reaches paths through arrays and variables too
        // and a `.join("meta")` check sees none of those. The one production
        // occurrence outside the two owners is a `[workspace]` CONFIG KEY that
        // happens to be spelled the same, so it is pinned by count: a second
        // one is a review, not a diff.
        let occurrences = code.matches("\"meta\"").count();
        let allowed = usize::from(name == "src/session_launch.rs");
        assert_eq!(
            occurrences, allowed,
            "{name} spells \"meta\" {occurrences} time(s); the file is named in src/store.rs \
             and an archive's copy of it in src/archive/store.rs"
        );
    }
}

#[test]
fn known_session_file_mutations_stay_with_the_store() {
    // What `src/telegram.rs`'s cursor proof rests on, checked rather than
    // asserted in prose: the locked append primitive is private, and the
    // retention rename is owned by the store. This conservative lexical
    // tripwire catches known direct write/rename forms before they can drift
    // outside that facade; it is not a complete data-flow analysis.
    // Controls FIRST: prove the tripwire catches both direct mutations before
    // asking it about the production tree. A planted line must make this test
    // red.
    for planted in [
        "fn planted(dir: &Path) { std::fs::write(crate::store::open(dir).events_path(), b\"\"); }",
        "fn planted(dir: &Path) { std::fs::rename(temp, crate::store::open(dir).events_path()).ok(); }",
    ] {
        assert!(
            direct_session_mutation(planted).is_some(),
            "the direct session mutation control was not detected"
        );
    }
    for (name, code) in product_halves() {
        if name == "src/store.rs" || name == "src/archive/store.rs" {
            continue;
        }
        assert!(
            direct_session_mutation(&code).is_none(),
            "{name} mutates a session container directly; use SessionStore"
        );
        assert!(
            !code.contains("append_locked"),
            "{name} reaches the locked-append primitive directly; \
             every known session-file mutation goes through SessionStore"
        );
    }
}

#[test]
fn the_claude_project_key_is_spelled_in_src_carry_alone() {
    // ONE OWNER of claude's project-folder rule: `carry::project_key`. The
    // resume probe, the rename preflight and the account carry all call it, so
    // a copy lands exactly where claude wrote the transcript and the probe
    // looks. `run::resumable` once spelled the rule inline — with only `/` as
    // `-`, which no claude folder ever matches for a dotted work dir — so the
    // guard is: nothing outside the owner flattens a path into a key, and the
    // probe is a CALLER. A TRIPWIRE over known shapes, not a proof: the ones
    // that have actually appeared plus the owner's own body (`len_utf16`,
    // `repeat_n('-'`), so a copy of it is named too. The behaviour itself is
    // pinned by the owner's unit tests.
    let halves = product_halves();
    let probe = halves
        .iter()
        .find(|(name, _)| name == "src/run.rs")
        .map_or("", |(_, code)| code.as_str());
    assert!(
        probe.contains("crate::carry::project_key("),
        "src/run.rs::resumable must call ae::carry::project_key, the one owner of claude's \
         project-folder rule"
    );
    for (name, code) in halves {
        if name == "src/carry.rs" {
            continue;
        }
        for shape in [
            "== '/' { '-'",
            ".replace('/', \"-\")",
            ".replace(\"/\", \"-\")",
            ".len_utf16()",
            "repeat_n('-'",
        ] {
            assert!(
                !code.contains(shape),
                "{name} spells the claude project key itself ({shape}); call ae::carry::project_key"
            );
        }
        assert!(
            !code
                .lines()
                .any(|line| line.contains("is_ascii_alphanumeric") && line.contains("else { '-' }")),
            "{name} spells the claude project key itself; call ae::carry::project_key"
        );
    }
}

#[test]
fn open_popup_and_run_shell_line_have_only_their_named_product_caller_and_pure_shell_module() {
    let halves = product_halves();
    for (leg, expected) in [
        (
            "open_popup(",
            &["src/console/term.rs", "src/transport.rs"][..],
        ),
        (
            "run_shell_line(",
            &["src/console/term.rs", "src/transport.rs"][..],
        ),
    ] {
        let mut callers: Vec<&str> = halves
            .iter()
            .filter(|(_, code)| code.contains(leg))
            .map(|(name, _)| name.as_str())
            .collect();
        callers.sort_unstable();
        assert_eq!(callers, expected, "{leg} product callers");
    }
    let shell = halves
        .iter()
        .find(|(name, _)| name == "src/console/shell.rs")
        .map_or_else(
            || panic!("src/console/shell.rs was not scanned"),
            |(_, code)| code,
        );
    for forbidden in ["transport", "doors", "lifecycle", "std::fs", "std::process"] {
        assert!(
            !shell.contains(forbidden),
            "src/console/shell.rs must stay pure, found {forbidden}"
        );
    }
    let term = halves
        .iter()
        .find(|(name, _)| name == "src/console/term.rs")
        .map_or_else(
            || panic!("src/console/term.rs was not scanned"),
            |(_, code)| code,
        );
    let shell_fn = &term[body_of(term, "fn shell(")];
    for writer in [
        "append_event",
        "append_memo",
        "publish_console_draft",
        "submit_ask",
        "admit_ask",
        "Carry",
        "recovery",
        "messages",
    ] {
        assert!(
            !shell_fn.contains(writer),
            "src/console/term.rs::shell must never record or send ! lines, found {writer}"
        );
    }
}
