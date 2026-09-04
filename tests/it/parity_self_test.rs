//! Self-tests for the parity harness — issue #93, stage 1.
//!
//! **Everything here is SYNTHETIC.** Every corpus is authored a few lines above
//! the test that uses it, and every lane is a `/bin/sh -c` one-liner written for
//! this file. Nothing reads `docs/migration/evidence/`, a seat annexe, or any
//! bash-produced output: stage 1 proves the PLUMBING, and it has to be able to
//! do that before there is any evidence worth trusting it with.
//!
//! The direction of judgement matters and is the reason these tests are not in
//! [`super::parity`]. A test may judge the harness. The harness may never judge
//! a lane — see [`the_harness_captures_and_never_judges`], which enforces that
//! structurally, by reading the harness's own source.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]
#![cfg(unix)]

use std::fs;
use std::io;
use std::marker::PhantomData;
use std::ops::Range;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use ae::json::{self, Value};

use super::parity::capture::raw::RawStatus;
use super::parity::capture::{ExitOutcome, LaneCapture, Manifest, PairedCapture};
use super::parity::{Corpus, Invocation, Lane, ScratchDir, run_pair};

/// A lane's evidence, read back the only way anything outside the capture
/// module can read it: off disk, from the artifacts the capture wrote.
///
/// This is not a workaround for the privacy boundary — it is the boundary
/// working. Stage 3 will read these same files, so a self-test that reads them
/// is testing what a consumer actually gets rather than an in-memory value no
/// consumer will ever hold.
struct Artifacts {
    dir: std::path::PathBuf,
}

impl Artifacts {
    fn of(run_root: &Path, lane: &str) -> Self {
        Self {
            dir: run_root.join(lane),
        }
    }

    fn path(&self, artifact: &str) -> std::path::PathBuf {
        self.dir.join(artifact)
    }

    fn bytes(&self, artifact: &str) -> Vec<u8> {
        fs::read(self.path(artifact))
            .unwrap_or_else(|err| panic!("{}: {artifact}: {err}", self.dir.display()))
    }

    fn text(&self, artifact: &str) -> String {
        String::from_utf8(self.bytes(artifact))
            .unwrap_or_else(|err| panic!("{artifact} is utf-8: {err}"))
    }

    fn stdout(&self) -> Vec<u8> {
        self.bytes("stdout")
    }

    fn stderr(&self) -> Vec<u8> {
        self.bytes("stderr")
    }

    fn exit(&self) -> String {
        self.text("exit")
    }

    fn command(&self) -> String {
        self.text("command.json")
    }

    fn clone_dir(&self) -> std::path::PathBuf {
        self.path("clone")
    }

    fn manifest(&self) -> ManifestArtifact {
        let parsed = json::parse(&self.text("manifest.json"))
            .unwrap_or_else(|err| panic!("manifest.json is one complete document: {err}"));
        let Value::Arr(entries) = parsed else {
            panic!("the manifest artifact is a JSON array")
        };
        ManifestArtifact { entries }
    }
}

/// The parsed `manifest.json`, queried the way the in-memory `Manifest` used
/// to be.
struct ManifestArtifact {
    entries: Vec<Value>,
}

impl ManifestArtifact {
    fn paths(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter_map(|entry| entry.get_str("path"))
            .collect()
    }

    fn entry(&self, path: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|entry| entry.get_str("path") == Some(path))
    }
}

/// A manifest entry's `mode`, back from its octal text.
fn mode_of(entry: &Value) -> u32 {
    let octal = entry
        .get_str("mode")
        .unwrap_or_else(|| panic!("a mode is recorded: {entry:?}"));
    u32::from_str_radix(octal, 8).unwrap_or_else(|err| panic!("the mode is octal: {err}"))
}

/// A `/bin/sh` one-liner. Authored here, for this file — never a producer.
fn sh(script: &str) -> Invocation {
    Invocation::new("/bin/sh").arg("-c").arg(script)
}

/// A lane whose command is a fixed script, ignoring the clone path.
fn script_lane(name: &str, script: &str) -> Lane {
    let script = script.to_owned();
    Lane::new(name, move |_clone| sh(&script))
}

/// The synthetic corpus: nested directories, a binary file, an executable bit,
/// and a symlink — the shapes an ae session directory actually has.
fn write_synthetic_corpus(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root.join("nested/deeper"))?;
    fs::write(root.join("alpha.txt"), b"alpha\n")?;
    fs::write(root.join("nested/beta.bin"), [0u8, 1, 2, 255])?;
    fs::write(root.join("nested/deeper/gamma.txt"), b"gamma\n")?;
    let executable = root.join("runme");
    fs::write(&executable, b"#!/bin/sh\nexit 0\n")?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
    std::os::unix::fs::symlink("alpha.txt", root.join("link-to-alpha"))?;
    Ok(())
}

/// A scratch dir holding `template/` (the corpus) and `run/` (the artifacts).
struct Bench {
    scratch: ScratchDir,
}

impl Bench {
    // `expect` is relaxed only inside `#[test]` bodies (clippy.toml); these are
    // helpers beside them, so they panic explicitly — same failure, same
    // loudness, one lint honoured.
    fn new(tag: &str) -> Self {
        let scratch =
            ScratchDir::new(tag).unwrap_or_else(|err| panic!("a scratch dir for {tag}: {err}"));
        write_synthetic_corpus(&scratch.path().join("template"))
            .unwrap_or_else(|err| panic!("the synthetic corpus: {err}"));
        Self { scratch }
    }

    fn corpus(&self) -> Corpus {
        Corpus::import(&self.scratch.path().join("template"))
            .unwrap_or_else(|err| panic!("the template imports: {err}"))
    }

    fn template(&self) -> std::path::PathBuf {
        self.scratch.path().join("template")
    }

    fn run_root(&self) -> std::path::PathBuf {
        self.scratch.path().join("run")
    }
}

#[test]
fn a_template_that_is_not_there_is_refused_rather_than_read_as_empty() {
    // An empty corpus produces two empty lanes that agree perfectly — the most
    // convincing wrong answer this harness could give.
    let scratch = ScratchDir::new("missing").expect("a scratch dir");
    let err = Corpus::import(&scratch.path().join("nope")).expect_err("must not resolve");
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[test]
fn a_template_that_is_a_file_is_refused() {
    let scratch = ScratchDir::new("notadir").expect("a scratch dir");
    let file = scratch.path().join("a-file");
    fs::write(&file, b"not a corpus").expect("write");
    let err = Corpus::import(&file).expect_err("must not resolve");
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn cloning_reproduces_the_tree_including_modes_and_links() {
    let bench = Bench::new("clone");
    let dest = bench.scratch.path().join("copy");
    bench.corpus().clone_to(&dest).expect("clones");

    assert_eq!(fs::read(dest.join("alpha.txt")).expect("read"), b"alpha\n");
    assert_eq!(
        fs::read(dest.join("nested/beta.bin")).expect("read"),
        [0u8, 1, 2, 255],
        "binary content survives byte for byte"
    );
    assert_eq!(
        fs::read(dest.join("nested/deeper/gamma.txt")).expect("read"),
        b"gamma\n",
        "the walk recurses past the first level"
    );

    let mode = fs::metadata(dest.join("runme"))
        .expect("stat")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755, "the executable bit is a parity fact");

    let link = fs::symlink_metadata(dest.join("link-to-alpha")).expect("stat");
    assert!(
        link.file_type().is_symlink(),
        "a symlink is copied as a link, not followed into a second copy of its target"
    );
    assert_eq!(
        fs::read_link(dest.join("link-to-alpha")).expect("readlink"),
        Path::new("alpha.txt")
    );
}

#[test]
fn each_lane_gets_its_own_clone_and_the_template_is_never_touched() {
    // If the lanes shared a directory, the second one would start from wherever
    // the first left it, and any later difference could be ordering rather than
    // behavior.
    let bench = Bench::new("isolation");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            script_lane("first", "rm alpha.txt && echo gone > only-here.txt"),
            script_lane("second", "cat alpha.txt"),
        ],
    )
    .expect("both lanes run");

    let first = Artifacts::of(&bench.run_root(), "first");
    let second = Artifacts::of(&bench.run_root(), "second");

    assert!(
        first.manifest().entry("alpha.txt").is_none(),
        "the first lane deleted it"
    );
    assert!(
        second.manifest().entry("alpha.txt").is_some(),
        "the second lane started from the template, not from the first lane's leftovers"
    );
    assert_eq!(second.stdout(), b"alpha\n", "and could still read it");
    assert!(
        first.manifest().entry("only-here.txt").is_some()
            && second.manifest().entry("only-here.txt").is_none(),
        "neither lane can see the other's writes"
    );
    assert!(
        bench.template().join("alpha.txt").exists(),
        "the template itself is read-only to this harness"
    );
}

#[test]
fn both_lanes_stdout_stderr_and_exit_code_are_captured_raw() {
    let bench = Bench::new("streams");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            script_lane(
                "noisy",
                "printf 'out-no-newline'; printf 'err\\n' >&2; exit 3",
            ),
            script_lane("quiet", "exit 0"),
        ],
    )
    .expect("both lanes run");

    let noisy = Artifacts::of(&bench.run_root(), "noisy");
    assert_eq!(
        noisy.stdout(),
        b"out-no-newline",
        "no newline is added, trimmed or normalised"
    );
    assert_eq!(noisy.stderr(), b"err\n", "the two streams stay apart");
    assert_eq!(noisy.exit(), "code 3\n");

    let quiet = Artifacts::of(&bench.run_root(), "quiet");
    assert!(quiet.stdout().is_empty());
    assert!(quiet.stderr().is_empty());
    assert_eq!(quiet.exit(), "code 0\n");
}

#[test]
fn a_failing_first_lane_does_not_cost_the_second_lane_its_evidence() {
    // "One lane refused to start" is itself a finding; aborting the run would
    // destroy the other side's evidence for it.
    let bench = Bench::new("failfirst");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            script_lane("broken", "exit 127"),
            script_lane("fine", "echo still-here"),
        ],
    )
    .expect("the pair completes even though a lane failed");

    assert_eq!(
        Artifacts::of(&bench.run_root(), "broken").exit(),
        "code 127\n"
    );
    assert_eq!(
        Artifacts::of(&bench.run_root(), "fine").stdout(),
        b"still-here\n"
    );
}

#[test]
fn a_first_lane_that_could_not_start_does_not_cost_the_second_lane_its_evidence() {
    // The case `exit 127` does NOT cover: that lane's process started. This one
    // never spawns, so the failure arrives before any artifact of its own —
    // and an early return here would take the second lane's evidence with it,
    // which is exactly what `run_pair` documents it will not do.
    let bench = Bench::new("nospawn");
    // `expect_err` would need `PairedCapture: Debug`, and it deliberately has
    // no such impl — a whole-value read is exactly the channel that is closed.
    // The let-else costs one line and buys the boundary.
    let Err(err) = run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            Lane::new("absent", |_clone| {
                Invocation::new("/nonexistent/ae-parity-no-such-program")
            }),
            script_lane("second", "printf ran > second-ran"),
        ],
    ) else {
        panic!("a lane that cannot be spawned is reported, not swallowed")
    };
    assert_eq!(err.kind(), io::ErrorKind::NotFound);

    let second = bench.run_root().join("second");
    assert!(
        second.join("clone/second-ran").is_file(),
        "the second lane never ran"
    );
    for artifact in ["stdout", "stderr", "exit", "manifest.json", "command.json"] {
        assert!(
            second.join(artifact).is_file(),
            "the second lane's {artifact} did not survive the first lane's failure"
        );
    }
    assert_eq!(
        fs::read_to_string(second.join("exit")).expect("read"),
        "code 0\n",
        "and it is captured as the lane that succeeded"
    );
}

#[test]
fn a_lane_killed_by_a_signal_is_recorded_as_signalled_not_as_a_code() {
    let bench = Bench::new("signal");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            script_lane("killed", "kill -9 $$"),
            script_lane("survivor", "exit 0"),
        ],
    )
    .expect("both lanes run");

    assert_eq!(
        Artifacts::of(&bench.run_root(), "killed").exit(),
        "signalled\n",
        "a capture that guessed a number here would be inventing evidence"
    );
}

#[test]
fn the_manifest_is_recursive_sorted_and_taken_after_the_run() {
    let bench = Bench::new("manifest");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            script_lane(
                "writer",
                "mkdir -p made/here && echo new > made/here/late.txt",
            ),
            script_lane("reader", "true"),
        ],
    )
    .expect("both lanes run");

    let writer = Artifacts::of(&bench.run_root(), "writer").manifest();
    let paths = writer.paths();

    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(
        paths, sorted,
        "directory-iteration order is arbitrary; the manifest is not"
    );

    for expected in [
        "alpha.txt",
        "link-to-alpha",
        "nested",
        "nested/beta.bin",
        "nested/deeper",
        "nested/deeper/gamma.txt",
        "runme",
        "made",
        "made/here",
        "made/here/late.txt",
    ] {
        assert!(
            paths.contains(&expected),
            "{expected} missing from {paths:?}"
        );
    }
    assert!(
        writer.entry("made/here/late.txt").is_some(),
        "the manifest describes what the lane LEFT, so it is taken after the run"
    );
}

#[test]
fn the_manifest_records_kind_size_mode_link_target_and_a_content_digest() {
    let bench = Bench::new("entries");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [script_lane("a", "true"), script_lane("b", "true")],
    )
    .expect("both lanes run");
    let manifest = Artifacts::of(&bench.run_root(), "a").manifest();

    let alpha = manifest.entry("alpha.txt").expect("the file is listed");
    assert_eq!(alpha.get_str("kind"), Some("file"));
    assert_eq!(alpha.get("len"), Some(&Value::Num(6)));
    assert_eq!(mode_of(alpha) & 0o777, 0o644);
    assert!(alpha.get_str("digest").is_some());
    assert_eq!(alpha.get("target"), None);

    let executable = manifest.entry("runme").expect("the executable is listed");
    assert_eq!(mode_of(executable) & 0o777, 0o755);

    let dir = manifest.entry("nested").expect("the directory is listed");
    assert_eq!(dir.get_str("kind"), Some("dir"));
    assert_eq!(
        dir.get("len"),
        None,
        "a directory has no content length to report"
    );
    assert!(dir.get_str("mode").is_some());

    let link = manifest
        .entry("link-to-alpha")
        .expect("the symlink is listed");
    assert_eq!(link.get_str("kind"), Some("symlink"));
    assert_eq!(link.get_str("target"), Some("alpha.txt"));
    assert_eq!(
        link.get("digest"),
        None,
        "a link's content is its target's, and this manifest does not follow it"
    );
}

#[test]
fn two_files_of_the_same_length_with_different_bytes_get_different_digests() {
    // The reason a digest is captured at all: length alone would render these
    // two identical, and a parity run comparing lengths would call them equal.
    let bench = Bench::new("digest");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            script_lane("left", "printf 'aaaa' > same-length.txt"),
            script_lane("right", "printf 'bbbb' > same-length.txt"),
        ],
    )
    .expect("both lanes run");

    let left = Artifacts::of(&bench.run_root(), "left").manifest();
    let right = Artifacts::of(&bench.run_root(), "right").manifest();
    let left = left.entry("same-length.txt").expect("listed");
    let right = right.entry("same-length.txt").expect("listed");

    assert_eq!(
        left.get("len"),
        right.get("len"),
        "the same length, deliberately"
    );
    assert_ne!(
        left.get_str("digest"),
        right.get_str("digest"),
        "and a different fingerprint"
    );
}

#[test]
fn the_artifacts_are_stored_side_by_side_and_hold_what_each_lane_produced() {
    let bench = Bench::new("artifacts");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            script_lane(
                "bash_like",
                "printf 'left\\n'; printf 'oops\\n' >&2; exit 4",
            ),
            script_lane("rust_like", "printf 'right\\n'"),
        ],
    )
    .expect("both lanes run");

    let root = bench.run_root();
    assert!(root.join("pair.json").is_file());
    let pair = fs::read_to_string(root.join("pair.json")).expect("read");
    assert!(
        pair.contains("\"lanes\":[\"bash_like\",\"rust_like\"]"),
        "{pair}"
    );
    assert!(pair.contains("template"), "{pair}");

    for lane in ["bash_like", "rust_like"] {
        let artifacts = Artifacts::of(&root, lane);
        for artifact in ["stdout", "stderr", "exit", "manifest.json", "command.json"] {
            assert!(
                artifacts.path(artifact).is_file(),
                "{lane}: {artifact} missing"
            );
        }
        assert!(
            artifacts.clone_dir().is_dir(),
            "the clone is KEPT, not cleaned up"
        );
    }

    // What the lane printed is what the artifact holds, byte for byte. With the
    // capture opaque, this file IS the evidence — there is no in-memory copy to
    // compare it against, and a later stage will read exactly these bytes.
    let left = Artifacts::of(&root, "bash_like");
    assert_eq!(left.stdout(), b"left\n");
    assert_eq!(
        left.stderr(),
        b"oops\n",
        "the two streams stay apart on disk"
    );
    assert_eq!(left.exit(), "code 4\n");
    assert_eq!(Artifacts::of(&root, "rust_like").stdout(), b"right\n");

    let manifest = left.text("manifest.json");
    assert!(
        manifest.starts_with('['),
        "the manifest is a JSON array: {manifest}"
    );
    assert!(manifest.contains("\"path\":\"alpha.txt\""), "{manifest}");
    assert!(manifest.contains("\"kind\":\"symlink\""), "{manifest}");
}

#[test]
fn the_command_is_recorded_beside_its_output() {
    let bench = Bench::new("command");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            Lane::new("cleared", |clone| {
                sh("printf '%s' \"${AE_PARITY_MARKER:-unset}\"")
                    .env_cleared()
                    .env("AE_PARITY_MARKER", "set-by-the-lane")
                    .env("AE_PARITY_CLONE", clone.to_string_lossy().into_owned())
            }),
            Lane::new("inherited", |_clone| {
                sh("printf '%s' \"${AE_PARITY_MARKER:-unset}\"")
            }),
        ],
    )
    .expect("both lanes run");

    let cleared = Artifacts::of(&bench.run_root(), "cleared");
    assert_eq!(
        cleared.stdout(),
        b"set-by-the-lane",
        "a lane's own environment survives the clear"
    );
    let recorded = cleared.command();
    assert!(recorded.contains("\"program\":\"/bin/sh\""), "{recorded}");
    assert!(recorded.contains("\"env_cleared\":true"), "{recorded}");
    assert!(recorded.contains("AE_PARITY_MARKER"), "{recorded}");
    assert!(
        recorded.contains(&cleared.clone_dir().to_string_lossy().into_owned()),
        "the clone path the lane was handed is recorded: {recorded}"
    );

    let inherited = Artifacts::of(&bench.run_root(), "inherited");
    assert_eq!(
        inherited.stdout(),
        b"unset",
        "and a lane that sets nothing sees nothing this harness invented"
    );
    assert!(
        inherited.command().contains("\"env_cleared\":false"),
        "{}",
        inherited.command()
    );
}

#[test]
fn a_lane_runs_in_its_own_clone_directory() {
    let bench = Bench::new("cwd");
    run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [
            script_lane("here", "pwd"),
            script_lane("also", "ls alpha.txt"),
        ],
    )
    .expect("both lanes run");

    let here = Artifacts::of(&bench.run_root(), "here");
    let printed = String::from_utf8(here.stdout()).expect("utf-8");
    // Compared through the filesystem: macOS resolves /var to /private/var, so
    // the string `pwd` prints is not always the string this harness built.
    assert_eq!(
        fs::canonicalize(printed.trim()).expect("the printed path exists"),
        fs::canonicalize(here.clone_dir()).expect("the clone exists")
    );
    assert_eq!(
        Artifacts::of(&bench.run_root(), "also").exit(),
        "code 0\n",
        "a relative path resolves against the clone"
    );
}

#[test]
fn two_lanes_may_not_share_a_name_because_they_would_share_a_directory() {
    let bench = Bench::new("dupname");
    let Err(err) = run_pair(
        &bench.corpus(),
        &bench.run_root(),
        [script_lane("same", "true"), script_lane("same", "true")],
    ) else {
        panic!("a collision must be refused, not silently overwritten")
    };
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_lane_name_that_is_not_an_ordinary_directory_component_is_refused() {
    let bench = Bench::new("badname");
    for bad in ["", "..", "a/b", "-leading", "with space", "dot.dot"] {
        let Err(err) = run_pair(
            &bench.corpus(),
            &bench.run_root(),
            [script_lane(bad, "true"), script_lane("ok", "true")],
        ) else {
            panic!("{bad:?} must be refused")
        };
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
    }
}

#[test]
fn a_manifest_of_an_empty_directory_is_empty_rather_than_an_error() {
    let scratch = ScratchDir::new("emptydir").expect("a scratch dir");
    let empty = scratch.path().join("empty");
    fs::create_dir(&empty).expect("mkdir");
    let manifest = Manifest::of(&empty).expect("an empty tree still walks");
    assert!(manifest.entries.is_empty());
    assert!(manifest.paths().is_empty());
}

#[test]
fn a_scratch_dir_removes_itself() {
    let path = {
        let scratch = ScratchDir::new("selfclean").expect("a scratch dir");
        fs::write(scratch.path().join("leftover"), b"x").expect("write");
        scratch.path().to_path_buf()
    };
    assert!(
        !path.exists(),
        "the scratch dir outlived its guard: {path:?}"
    );
}

// ---------------------------------------------------------------------------
// The value-level channel, closed at the type system.
//
// A textual guard reads names. The channel below has none: `Debug` reads every
// field of a capture at once, and `PartialEq` compares every field at once, so
// judgement can be written without ever spelling `stdout`. No pattern, however
// refined, sees that — which is why it is closed by the type system instead, and
// why the mechanisms are layered:
//
//   * PRIVACY stops a read from outside `capture`. The compiler enforces it on
//     every build, and the self-tests below feel it: they read artifacts off
//     disk because there is no other way in.
//   * `#[expect(dead_code)]` on each evidence field stops a read from INSIDE
//     `capture` too — reading one makes the expectation unfulfilled, which
//     `-D warnings` turns into a build failure (measured, not assumed).
//   * NEITHER of those sees a derive: a derived `Debug` reads every field and
//     does NOT unfulfil the expectation (measured — derived impls are
//     special-cased by the dead-code lint). That is this probe's job.
// ---------------------------------------------------------------------------

/// Reviewer4's THIRD injection, from the round-3 delta, byte for byte.
///
/// Kept although it can no longer be spliced anywhere: it is the artifact that
/// says WHY the capture types have no `Debug`. Pasting it into `run_pair` now
/// fails to compile — `PairedCapture` and `LaneCapture` have no such impl for
/// `{first:?}` to call — which is the strongest form this must-red can take.
const WHOLE_VALUE_DEBUG_COMPARISON: &str =
    "    if std::env::var_os(\"AE_PARITY_DEBUG_JUDGE\").is_some()
        && format!(\"{first:?}\") != format!(\"{second:?}\")
    {
        return Err(io::Error::other(\"lane captures differ\"));
    }
";

/// Reviewer4's round-4 injections, from their tree, byte for byte.
///
/// Both judge the RAW `std::process::Output` inside `capture_lane`, before it
/// ever becomes an opaque capture — earlier than anything the previous rounds
/// closed. Neither could be caught by a name rule: `status` is the raw spelling
/// of what is stored as `exit`, so no list derived from `LaneCapture` contains
/// it; and `std::process::Output` implements `Debug` natively, so no probe of
/// the capture types can reach it.
///
/// Both begin `let output = command.output()?;`, and that line is why they are
/// now unbuildable rather than uncaught: this harness runs a child in exactly
/// one place, wrapped there, and
/// [`a_child_process_is_run_in_exactly_one_place_and_is_wrapped_there`] pins it.
const RAW_STATUS_JUDGEMENT: &str = "        let output = command.output()?;
        if std::env::var_os(\"AE_PARITY_STATUS_JUDGE\").is_some() && !output.status.success() {
            return Err(io::Error::other(\"lane status differed\"));
        }
";

/// The second of the pair. See [`RAW_STATUS_JUDGEMENT`].
const RAW_DEBUG_JUDGEMENT: &str = "        let output = command.output()?;
        if std::env::var_os(\"AE_PARITY_RAW_OUTPUT_JUDGE\").is_some()
            && format!(\"{output:?}\") != \"expected\"
        {
            return Err(io::Error::other(\"lane output differed\"));
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
    // `src/upgrade.rs` is the second `exec` and arrived with slice Z3, when
    // `ae-entry` — which used to run the installer — was deleted: `ae upgrade`
    // BECOMES the immutable sibling installer, so the installer's exit status
    // is ae's and no surviving parent can misreport a repair.
    // All three are listed rather than exempted because the value of this guard
    // is that adding a door is a line in a review, not a diff nobody read.
    assert_eq!(
        sites,
        vec![
            "src/run.rs".to_owned(),
            "src/transport.rs".to_owned(),
            "src/upgrade.rs".to_owned(),
            "tests/it/cli.rs".to_owned(),
            "tests/it/parity.rs".to_owned(),
            "tests/it/parity_self_test.rs".to_owned(),
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
    // An eleventh entry is red. A relaxation this counter CANNOT see is not —
    // that is what the semantic guard above is for.
    assert_eq!(
        inventory,
        vec![
            ("src/run.rs".to_owned(), 1),
            ("src/transport.rs".to_owned(), 1),
            ("src/upgrade.rs".to_owned(), 1),
            ("tests/it/cli.rs".to_owned(), 5),
            ("tests/it/parity.rs".to_owned(), 1),
            ("tests/it/parity_self_test.rs".to_owned(), 1),
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

/// Does `T` implement a given trait? Asked of the compiler, answered at run time.
///
/// Inherent methods are selected before trait methods, so an inherent `is_*`
/// wins exactly when its bound holds; otherwise the blanket trait method
/// answers. Stable, and no dependency — which matters here, because the
/// alternative (a `compile_fail` doctest) cannot reach these types at all:
/// rustdoc does not collect doctests from an integration-test target (measured:
/// a deliberately failing doctest planted in `parity.rs` was never run).
struct Probe<T>(PhantomData<T>);

trait NoDebug {
    fn is_debug(&self) -> bool {
        false
    }
}
impl<T> NoDebug for Probe<T> {}
impl<T: std::fmt::Debug> Probe<T> {
    #[expect(
        clippy::unused_self,
        reason = "the receiver is what makes inherent-method priority pick the bounded impl"
    )]
    fn is_debug(&self) -> bool {
        true
    }
}

trait NoPartialEq {
    fn is_partial_eq(&self) -> bool {
        false
    }
}
impl<T> NoPartialEq for Probe<T> {}
impl<T: PartialEq> Probe<T> {
    #[expect(
        clippy::unused_self,
        reason = "the receiver is what makes inherent-method priority pick the bounded impl"
    )]
    fn is_partial_eq(&self) -> bool {
        true
    }
}

trait NoHash {
    fn is_hash(&self) -> bool {
        false
    }
}
impl<T> NoHash for Probe<T> {}
impl<T: std::hash::Hash> Probe<T> {
    #[expect(
        clippy::unused_self,
        reason = "the receiver is what makes inherent-method priority pick the bounded impl"
    )]
    fn is_hash(&self) -> bool {
        true
    }
}

trait NoOrd {
    fn is_ord(&self) -> bool {
        false
    }
}
impl<T> NoOrd for Probe<T> {}
impl<T: PartialOrd> Probe<T> {
    #[expect(
        clippy::unused_self,
        reason = "the receiver is what makes inherent-method priority pick the bounded impl"
    )]
    fn is_ord(&self) -> bool {
        true
    }
}

#[test]
fn the_capture_types_carry_no_whole_value_channel() {
    // The positive control comes FIRST and is not optional: a probe that could
    // never answer "yes" would report every derive as absent and pass forever.
    // That is the same vacuous-gate failure the guard's own control runs exist
    // to prevent.
    let control = Probe::<String>(PhantomData);
    assert!(control.is_debug(), "the probe cannot detect Debug at all");
    assert!(control.is_partial_eq(), "the probe cannot detect PartialEq");
    assert!(control.is_hash(), "the probe cannot detect Hash");
    assert!(control.is_ord(), "the probe cannot detect PartialOrd");

    let lane = Probe::<LaneCapture>(PhantomData);
    assert!(
        !lane.is_debug(),
        "LaneCapture implements Debug — a whole-value read of every field, and \
         exactly the channel {WHOLE_VALUE_DEBUG_COMPARISON:?} was written through"
    );
    assert!(
        !lane.is_partial_eq(),
        "LaneCapture implements PartialEq — a whole-value verdict needing no field name"
    );
    assert!(
        !lane.is_hash(),
        "LaneCapture implements Hash — two fingerprints compare"
    );
    assert!(
        !lane.is_ord(),
        "LaneCapture implements PartialOrd — an ordering is a comparison"
    );

    let paired = Probe::<PairedCapture>(PhantomData);
    assert!(
        !paired.is_debug(),
        "PairedCapture implements Debug — one format! reads BOTH lanes at once"
    );
    assert!(
        !paired.is_partial_eq(),
        "PairedCapture implements PartialEq"
    );
    assert!(!paired.is_hash(), "PairedCapture implements Hash");
    assert!(!paired.is_ord(), "PairedCapture implements PartialOrd");

    // Clone is deliberately NOT probed: cloning a capture yields another opaque
    // capture, and reading it still requires one of the channels above. The
    // absent ones are the ones that hand over the contents.
}

#[test]
fn the_raw_status_wrapper_carries_no_whole_value_channel_either() {
    // The control here is unusually good, because it is the very thing being
    // wrapped: `std::process::ExitStatus` and `std::process::Output` DO
    // implement Debug. That is what made reviewer4's round-4 injection compile,
    // and it proves this probe is reading the world rather than always
    // answering "no".
    assert!(
        Probe::<std::process::ExitStatus>(PhantomData).is_debug(),
        "the std type this wraps has Debug — if this is false the probe is broken"
    );
    assert!(
        Probe::<std::process::Output>(PhantomData).is_debug(),
        "and so does Output, which is what {RAW_DEBUG_JUDGEMENT:?} formatted"
    );

    let raw = Probe::<RawStatus>(PhantomData);
    assert!(
        !raw.is_debug(),
        "RawStatus implements Debug — `format!(\"{{status:?}}\")` reads the status whole"
    );
    assert!(!raw.is_partial_eq(), "RawStatus implements PartialEq");
    assert!(!raw.is_hash(), "RawStatus implements Hash");
    assert!(!raw.is_ord(), "RawStatus implements PartialOrd");

    // The outcome it converts to is just as bare: nothing here may compare an
    // outcome against an expectation, only write it and store it.
    let outcome = Probe::<ExitOutcome>(PhantomData);
    assert!(!outcome.is_debug(), "ExitOutcome implements Debug");
    assert!(!outcome.is_partial_eq(), "ExitOutcome implements PartialEq");
}

// ---------------------------------------------------------------------------
// The source scanner — DEFENCE IN DEPTH, and no longer the claim.
//
// Read this before reading what it does. The load-bearing guarantees are in the
// TYPE SYSTEM and are listed in `parity.rs`'s module docs: the output bytes
// never enter the process, the capture types are opaque with no whole-value
// impls, `#[expect(dead_code)]` fails the build on an in-module read, and
// `std::process::Command` is a denied type with one pinned door. Those are
// enforced by the compiler on every build.
//
// What follows is a heuristic over the source text. It earns its place by
// catching shapes the compiler is not asked about — a judgement written between
// the sites where evidence legitimately lives — and it has caught four real
// injections. It is NOT what makes the harness trustworthy, and every time it
// has been treated as such it has been laundered past: by an unlisted spelling,
// by an alias, by a whole-value read with no field name in it. Each of those is
// now closed by a type, not by a pattern here.
//
// Keep it. Do not promote it back.
//
// The scope boundary, enforced structurally rather than remembered: stage 1
// CAPTURES. A harness that knows what the answer should be is a harness whose
// captures were filtered through that belief before anyone read them.
//
// The guard reads the harness's own source. What it does NOT do any more is
// work from a list of forbidden spellings: a comparison written without the
// word `assert` is still a comparison, and the first version of this guard
// passed a dormant `first.stdout != second.stdout` — see
// [`DORMANT_REAL_LANE_COMPARISON`], which is kept here as a must-red case —
// while the whole integration target stayed green. What it looks for now is the
// SHAPE of judgement:
//
//   1. the spellings of a test          — kept; they are still true violations;
//   2. evidence named outside the one SITE it enters at — `capture_lane`'s own
//      body, plus the declaration of the type that holds it. The field list is
//      READ OUT of `struct LaneCapture` itself rather than listed here, and the
//      exemption is a place rather than an identifier: an earlier version
//      trusted the receiver name `output`, and rebinding a capture to a local
//      of that name laundered a real comparison past it;
//   3. a branch taken on evidence       — `if`/`match`/`while` in a statement
//      that touches it;
//   4. a comparison touching evidence   — `==`, `!=`, `.eq(`, `.cmp(`, …;
//   5. an error raised or a panic beside it;
//   6. the incumbent's lane names       — a scope rule rather than a judgement
//      one: stage-1 plumbing cannot know stage 3's lane names, and the
//      injection above was DORMANT precisely because it keyed on them.
//
// Rule 2 is the load-bearing one, and it is a SITE rule: outside
// `capture_lane`'s body, a lane's stdout, stderr, exit or manifest may not be
// named at all. Rebinding, aliasing and destructuring are all reads of a name
// somewhere, and no rebinding outside that body can move the body. Rules 3-5
// are not decoration on top of it: INSIDE the site, where naming evidence is
// what the site is for, judgement is caught by its shape alone. The rules
// overlap deliberately — each is proven able to go red on its own below.
// ---------------------------------------------------------------------------

/// Reviewer4's injection from the slice-2 cross-model review, verbatim.
///
/// Kept exactly as it was written rather than paraphrased: this is the line
/// that passed the spelling-based guard AND the full integration target, so a
/// tidied-up version would prove only that the guard catches the tidy version.
const DORMANT_REAL_LANE_COMPARISON: &str = "        if first.lane == \"bash\" && second.lane == \"rust\" && first.stdout != second.stdout { return Err(io::Error::other(\"lane outputs differ\")); }\n";

/// Reviewer4's SECOND injection, from the round-2 delta, byte for byte.
///
/// The first version of this guard exempted the receiver name `output` — the
/// raw child output, the one thing at capture time that is not yet a capture.
/// This rebinds a finished `LaneCapture` to a local of that name, twice, and
/// launders both lanes' stdout out through it before comparing them; the env
/// gate keeps it dormant, so the full integration target stayed green too.
/// It is the reason rule 2 keys on a SITE and not on an identifier.
const ALIAS_LAUNDERED_COMPARISON: &str = "    if std::env::var_os(\"AE_PARITY_JUDGE\").is_some() {
        let output = &first;
        let left = &output.stdout;
        let output = &second;
        let right = &output.stdout;
        if left != right {
            return Err(io::Error::other(\"lane outputs differ\"));
        }
    }
";

/// Where a judge would go in `run_pair`: after both lanes have been captured.
const AFTER_BOTH_CAPTURES: &str = "    Ok(PairedCapture::of(";

/// Where a judge would go in `capture_lane`: with the raw child output in hand.
const AT_CAPTURE_TIME: &str = "        Ok(LaneCapture {";

/// The harness's own source — the file this guard protects.
fn harness_source() -> String {
    // `expect` is relaxed only inside `#[test]` bodies (clippy.toml); these are
    // helpers beside them, so they panic explicitly.
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/parity.rs"))
        .unwrap_or_else(|err| panic!("the harness source is readable: {err}"))
}

/// `source` with `injection` spliced in just before `anchor`.
fn inject(source: &str, anchor: &str, injection: &str) -> String {
    assert!(
        source.contains(anchor),
        "the injection point {anchor:?} moved — this guard's red-proof would be testing nothing"
    );
    source.replacen(anchor, &format!("{injection}{anchor}"), 1)
}

/// The harness with `injection` spliced into `run_pair`.
fn harness_with(injection: &str) -> String {
    inject(&harness_source(), AFTER_BOTH_CAPTURES, injection)
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether `word` occurs in `text` as a whole word.
fn mentions(text: &str, word: &str) -> bool {
    !word_offsets(text, word).is_empty()
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

/// The fields a lane capture holds, read out of the harness's own
/// `struct LaneCapture`.
///
/// Derived rather than listed, because a guard whose list has drifted from the
/// type it guards has already stopped working: a field added tomorrow is
/// covered without anyone remembering this file exists.
fn observation_fields(code: &str) -> Vec<String> {
    // The extent comes from the same brace matching the site rule uses. An
    // earlier version looked for a `}` in the first column, which was true only
    // while the type sat at the top level: once it moved into `capture`, that
    // search ran past its real end and swallowed the NEXT type's fields into the
    // evidence list. One mechanism, so there is one thing to be right about.
    let anchor = "struct LaneCapture";
    let declaration = body_of(code, anchor);
    let mut fields = Vec::new();
    for line in code[declaration.start + 1..declaration.end - 1].lines() {
        let line = line
            .trim()
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub ");
        let Some((name, _)) = line.split_once(':') else {
            continue;
        };
        if name.is_empty() || !name.bytes().all(is_ident_byte) {
            continue;
        }
        // `lane` is the caller's LABEL for a side, not something the lane
        // produced — refusing a name collision is how `run_pair` opens, and the
        // word is a parameter name in nearly every function here. Excluded
        // deliberately and narrowly: every field carrying what a lane DID stays.
        if name == "lane" {
            continue;
        }
        fields.push(name.to_owned());
    }

    for required in ["exit", "stdout", "stderr", "manifest"] {
        assert!(
            fields.iter().any(|field| field == required),
            "the evidence list derived from `{anchor}` has no {required}: {fields:?} — \
             the guard would be vacuous"
        );
    }
    fields
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

/// Every mention of an evidence field outside the two sites evidence may live in.
///
/// **The exemption is a PLACE, not a name.** Evidence enters this harness at
/// three sites and no others: `mod raw`, which wires the child's streams to
/// their files; `capture_lane`'s body, which names those files and the manifest;
/// and the declaration of `struct LaneCapture`, which says what a capture holds. A field named anywhere else is the
/// harness reading back what it captured, which is the first step of every
/// judgement however it is spelled afterwards.
///
/// The previous version of this rule trusted the identifier `output` as the
/// receiver, and rebinding a capture to a local of that name laundered a real
/// cross-lane comparison straight past it. A name can be rebound; a site cannot.
/// Keying on the site also closes reads that never spell a field access at all:
/// `let LaneCapture { stdout: left, .. } = first;` names the field just the
/// same, and a receiver scan would never have seen it.
fn evidence_outside_its_site(code: &str, with_literals: &str, fields: &[String]) -> Vec<String> {
    assert_eq!(
        code.len(),
        with_literals.len(),
        "the two views must share one set of offsets"
    );
    let declaration = body_of(code, "struct LaneCapture");
    let capture_site = body_of(code, "fn capture_lane");
    // The third site, and the innermost: `raw` is where the child's streams are
    // wired to their files, so `Command::stdout`/`stderr` are named there by the
    // std API itself. It is admitted as a SITE for the same reason as the other
    // two — it is where evidence enters — and it is the smallest of them: a
    // newtype and one function, holding no bytes at all.
    let wiring_site = body_of(code, "mod raw");
    let mut escaped = Vec::new();
    for field in fields {
        // Searched WITH the literals in place: `join("stdout")` outside the
        // capture site is the harness re-reading an artifact it wrote, which is
        // the same read by a longer route. Inside the site it is the write, and
        // the write IS the capture.
        for at in word_offsets(with_literals, field) {
            if declaration.contains(&at) || capture_site.contains(&at) || wiring_site.contains(&at)
            {
                continue;
            }
            escaped.push(format!(
                "evidence `{field}` is named outside the capture site: {}",
                squashed(line_at(with_literals, at))
            ));
        }
    }
    escaped
}

/// Every byte offset at which `word` occurs in `code` as a whole word.
fn word_offsets(code: &str, word: &str) -> Vec<usize> {
    let bytes = code.as_bytes();
    let mut offsets = Vec::new();
    let mut from = 0;
    while let Some(found) = code[from..].find(word) {
        let at = from + found;
        from = at + word.len();
        let before = at.checked_sub(1).map(|index| bytes[index]);
        let after = bytes.get(at + word.len()).copied();
        if !before.is_some_and(is_ident_byte) && !after.is_some_and(is_ident_byte) {
            offsets.push(at);
        }
    }
    offsets
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

/// Every way this guard knows to say "this source JUDGES". Empty means clean.
fn judgment_violations(source: &str) -> Vec<String> {
    let decommented = strip_comments(source);
    let code = strip_literals(&decommented);
    let fields = observation_fields(&code);
    let mut found = Vec::new();

    // 1 — the spellings of a test.
    for spelling in [
        "#[test]",
        "assert!",
        "assert_eq!",
        "assert_ne!",
        "debug_assert",
        "panic!",
        "unreachable!",
        "todo!",
    ] {
        if code.contains(spelling) {
            found.push(format!(
                "the harness must not {spelling} — it captures, it never judges"
            ));
        }
    }

    // 2 — evidence named outside the one site it enters at.
    found.extend(evidence_outside_its_site(&code, &decommented, &fields));

    // 3, 4, 5 — judgement by shape, statement by statement.
    for chunk in code.split(['{', '}', ';']) {
        let touched: Vec<&str> = fields
            .iter()
            .filter(|field| mentions(chunk, field))
            .map(String::as_str)
            .collect();
        if touched.is_empty() {
            continue;
        }
        if ["if", "match", "while"]
            .iter()
            .any(|keyword| mentions(chunk, keyword))
        {
            found.push(format!(
                "the harness branches on captured evidence {touched:?}: {}",
                squashed(chunk)
            ));
        }
        if [
            "==",
            "!=",
            "<=",
            ">=",
            ".eq(",
            ".ne(",
            ".lt(",
            ".gt(",
            ".cmp(",
            ".partial_cmp(",
            ".eq_ignore_ascii_case(",
            ".starts_with(",
            ".ends_with(",
            ".contains(",
        ]
        .iter()
        .any(|operator| chunk.contains(operator))
        {
            found.push(format!(
                "the harness compares captured evidence {touched:?}: {}",
                squashed(chunk)
            ));
        }
        if chunk.contains("Err(") {
            found.push(format!(
                "the harness raises an error beside captured evidence {touched:?}: {}",
                squashed(chunk)
            ));
        }
    }

    // 6 — the incumbent's lane names. Read from the source WITH its literals,
    // because that is where a dormant rule keys on them.
    for name in ["bash", "rust"] {
        if mentions(&decommented, name) {
            found.push(format!(
                "the harness names the {name} lane — stage-1 plumbing cannot know stage 3's \
                 lane names, and a rule keyed on one is judgement waiting for its corpus"
            ));
        }
    }

    found
}

#[test]
fn the_harness_captures_and_never_judges() {
    let violations = judgment_violations(&harness_source());
    assert!(
        violations.is_empty(),
        "the harness judges its lanes: {violations:#?}"
    );
}

#[test]
fn the_guard_is_red_for_the_dormant_comparison_that_slipped_past_its_first_version() {
    // The control run. A guard that cannot fail proves nothing, and THIS is the
    // case it has to fail on: real lane names, an inequality between two lanes'
    // stdout, an error returned on the difference — dormant only because no
    // synthetic lane is ever called `bash`.
    let violations = judgment_violations(&harness_with(DORMANT_REAL_LANE_COMPARISON));
    assert!(
        !violations.is_empty(),
        "the injection that started all this stayed green"
    );
}

#[test]
fn the_guard_is_red_for_the_alias_laundered_comparison_that_slipped_past_round_two() {
    // The second control run, at reviewer4's own anchor: immediately after both
    // results are unwrapped and immediately before the pair is handed back.
    // Round 1 caught the comparison because it named `first.stdout`; this one
    // never does — it renames both captures to the one receiver the old rule
    // trusted. Nothing about the SHAPE of the comparison gives it away either:
    // `left != right` mentions no field at all.
    let violations = judgment_violations(&harness_with(ALIAS_LAUNDERED_COMPARISON));
    assert!(
        !violations.is_empty(),
        "the alias-laundered comparison stayed green"
    );
}

#[test]
fn the_guard_is_red_for_every_shape_of_judgement_it_claims_to_stop() {
    for (shape, injection) in [
        (
            "a comparison spelled as an assertion",
            "        assert_eq!(first.stdout, second.stdout);\n",
        ),
        (
            "a test that moved into the harness",
            "        #[test]\n        fn compare() {}\n",
        ),
        (
            "a branch on evidence with no comparison in it",
            "        if first.stdout.is_empty() { return Err(io::Error::other(\"a silent lane\")); }\n",
        ),
        (
            "a comparison laundered through locals",
            "        let left = first.stdout.clone();\n        let right = second.stdout.clone();\n        if left != right { return Err(io::Error::other(\"differ\")); }\n",
        ),
        (
            "a verdict reached through the manifest instead of the streams",
            "        if first.manifest.paths() != second.manifest.paths() { return Err(io::Error::other(\"trees differ\")); }\n",
        ),
        (
            "evidence taken by destructuring instead of by field access",
            "        let LaneCapture { stdout: left, .. } = &first;\n        let LaneCapture { stdout: right, .. } = &second;\n        if left != right { return Err(io::Error::other(\"differ\")); }\n",
        ),
        (
            "evidence read back without judging it yet",
            "        let _peek = second.exit;\n",
        ),
    ] {
        let violations = judgment_violations(&harness_with(injection));
        assert!(!violations.is_empty(), "{shape} stayed green");
    }
}

#[test]
fn the_scanner_is_red_for_a_judgement_written_inside_the_capture_site() {
    // SCANNER-ONLY, and labelled so rather than quietly left to read as more
    // than it is: this injection names an `output` binding that NO LONGER
    // EXISTS — the bytes never enter the process — so it cannot be compiled
    // against the live harness the way the other red-proofs can. It exercises
    // the text rules and nothing else.
    //
    // What it still proves is worth keeping: inside a site, where naming
    // evidence is what the site is FOR, a judgement is caught by its SHAPE
    // alone rather than by the receiver rule. The BUILDABLE proof for this
    // region is the capability boundary — see
    // `the_lint_relaxations_this_counter_can_see_are_the_expected_ones`.
    let judged = inject(
        &harness_source(),
        AT_CAPTURE_TIME,
        "        if output.stdout != b\"expected\" { return Err(io::Error::other(\"unexpected stdout\")); }\n",
    );
    let violations = judgment_violations(&judged);
    assert!(
        !violations.is_empty(),
        "a judge at capture time stayed green"
    );
    assert!(
        violations
            .iter()
            .all(|violation| !violation.contains("outside the capture site")),
        "inside the site, naming evidence is what the site is FOR — this case has \
         to be caught by shape alone: {violations:#?}"
    );
}

#[test]
fn the_guard_reads_code_and_not_prose() {
    // The stripper is why the harness's docs can SAY what it must never do, and
    // why `join("stdout")` can name an artifact file. It is also the thing that
    // could silently eat the code instead — hence the sentinel.
    let commented = harness_with(&format!(
        "        // {}\n",
        DORMANT_REAL_LANE_COMPARISON.trim()
    ));
    assert!(
        judgment_violations(&commented).is_empty(),
        "the guard read a comment as the violation"
    );
    assert!(
        strip_literals(&strip_comments(&harness_source())).contains("fn run_pair"),
        "the stripper dropped the code as well as the comments"
    );
}

#[test]
fn the_evidence_list_is_the_capture_types_own_fields() {
    let code = strip_literals(&strip_comments(&harness_source()));
    for expected in ["exit", "stdout", "stderr", "manifest"] {
        assert!(
            observation_fields(&code).iter().any(|f| f == expected),
            "{expected} missing"
        );
    }

    // Derivation proven end to end: a field this file has never heard of is
    // guarded the moment the capture type declares it.
    let widened = harness_source().replacen(
        "pub(crate) struct LaneCapture {",
        "pub(crate) struct LaneCapture {\n        timings: Vec<u64>,",
        1,
    );
    assert!(
        observation_fields(&strip_literals(&strip_comments(&widened)))
            .iter()
            .any(|field| field == "timings"),
        "a newly declared capture field was not picked up"
    );
    let judged = inject(
        &widened,
        AFTER_BOTH_CAPTURES,
        "        if first.timings != second.timings { return Err(io::Error::other(\"slower\")); }\n",
    );
    assert!(
        !judgment_violations(&judged).is_empty(),
        "the derived field was picked up but not enforced"
    );
}
