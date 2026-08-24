//! The two helper read surfaces against the FROZEN CORPUS, row by row.
//!
//! # What makes this different from the unit suites
//!
//! `src/requests.rs` and `src/events_tail.rs` test their rules against fixtures
//! they author. This module authors nothing. It walks the 206 opaque P1 rows of
//! `docs/migration/evidence/corpus/INVOCATIONS.tsv`, reads each row's FIXTURE
//! bytes and its CAPTURED expected bytes out of the frozen tree, and compares
//! byte for byte. Neither side is written here, so a rule this crate got wrong
//! cannot be made to agree with itself.
//!
//! **The corpus is read and never written.** No file under
//! `docs/migration/evidence/` is opened for writing, created or removed by this
//! module; a capture that looks wrong is a FINDING for the seats and never an
//! edit. Frozen means frozen.
//!
//! # What it proves, and what it cannot
//!
//! For `helper:requests` it compares stdout, stderr and the process status
//! against the row's recorded `rc` — the whole observable surface.
//!
//! For `helper:events-tail` it compares STDOUT ONLY, and that limit is a
//! finding rather than a gap in the test. All 38 of those rows were captured by
//! killing a `tail -f` after four seconds (`rc=143`, `bounded=4s=yes`), so their
//! recorded stderr is GNU bash's own job-control notification naming the
//! pipeline it killed. It is not reproducible by a non-bash implementation, and
//! the corpus is not even self-consistent about it: 37 rows carry one byte
//! string and `arms/A1/c09-dupkey-unknown-ro` carries the same message truncated
//! after its first line. See `crate::events_tail`'s module docs.
//!
//! # The argv mapping this module declares
//!
//! ```text
//! <AE_HOME>/sessions/<name>/requests    [mine|inbox|all]
//!   -> ae _requests    <AE_HOME>/sessions/<name> [mine|inbox|all]
//! <AE_HOME>/sessions/<name>/events-tail
//!   -> ae _events-tail <AE_HOME>/sessions/<name>
//! ```
//!
//! A parity run must declare that mapping as a fixed input (phase-4 criterion
//! 14). This module exercises the LIBRARY behind it; `super::cli` exercises the
//! argv itself through the real binary.

#![allow(
    clippy::disallowed_methods,
    reason = "this module's whole job is reading the frozen corpus off disk; the \
              boundary is about what PRODUCT code may reach"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ae::events_tail;
use ae::requests::{self, Mode, Viewer};

/// How many rows of each surface the corpus holds. Asserted, not assumed: a
/// comparison over a population that quietly shrank is a green run that proved
/// less than the last one.
const REQUESTS_ROWS: usize = 168;
const EVENTS_TAIL_ROWS: usize = 38;

/// The rows that cannot be replayed from fixture bytes, and WHY.
///
/// Exactly three, all in `arms/A1/c20-405k-live` — its `s0-baseline`,
/// `s1-extra-pane` and `s2-extra-pane-and-missing-roster-pane` states. That case
/// records `clone_mode=live` and no `template=` at all: its capture ran against
/// a live tmux session, so there are no fixture bytes to replay and its own
/// closer is a criterion-11 product-valid live arm, not this comparison.
///
/// Named and COUNTED rather than filtered. The count is asserted, so a fourth
/// unreplayable row fails this test instead of quietly shrinking the population
/// it claims to have compared — which is how a first-line-only `case.txt` reader
/// was caught here reporting 62.
const UNREPLAYABLE_ROWS: usize = 3;

/// Rows whose captured stderr is `grep(1)`'s, not `ae`'s — see
/// [`the_meta_bootstrap_grep_noise_is_not_ae_output`] for the proof and the
/// finding. Their stdout and rc still compare exactly; only the stderr
/// comparison is held out, and the count is asserted so an eleventh cannot
/// join them unnoticed.
const GREP_NOISE_ROWS: usize = 10;

/// The capture host's scratch root. Every byte of the grep noise names a path
/// under it, which is why no successor run on any other machine can produce
/// those bytes.
const CAPTURE_SCRATCH_ROOT: &str = "/tmp/aecx/";

fn evidence() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("migration")
        .join("evidence")
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn read_text(path: &Path) -> String {
    String::from_utf8(read(path)).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// One invocation row, resolved against the artifacts it needs.
struct Row {
    /// The case directory, relative to `batch-c-artifacts/`.
    case: PathBuf,
    /// The `out/<consumer>.*` basename.
    consumer: String,
    /// `helper:requests` or `helper:events-tail`.
    surface: String,
    /// The exit status the frozen capture recorded.
    rc: i32,
    /// `<group>/<member>` from the case's `template=`, empty when live.
    template: String,
    /// The session whose meta directory the helper was invoked in.
    session: String,
    /// The whole normalised argv, verbatim.
    argv: String,
}

impl Row {
    /// The fixture meta directory: `templates/<group>/fixture-bytes/<member>/sessions/<session>`.
    fn meta_dir(&self, root: &Path) -> Option<PathBuf> {
        let (group, member) = self.template.split_once('/')?;
        Some(
            root.join("batch-c-artifacts")
                .join("templates")
                .join(group)
                .join("fixture-bytes")
                .join(member)
                .join("sessions")
                .join(&self.session),
        )
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        root.join("batch-c-artifacts").join(&self.case).join("out")
    }

    /// The captured stdout. An absent file is zero bytes — the capture harness
    /// writes one per stream and a stream that said nothing leaves none.
    fn captured(&self, root: &Path, stream: &str) -> Vec<u8> {
        let path = self
            .out_dir(root)
            .join(format!("{}.{stream}", self.consumer));
        std::fs::read(&path).unwrap_or_default()
    }

    /// The mode token, taken from the argv's tail as the frozen `$1`.
    fn mode(&self) -> Option<Mode> {
        Mode::parse(self.argv.split_whitespace().nth(1))
    }

    fn label(&self) -> String {
        format!("{} :: {}", self.case.display(), self.consumer)
    }

    /// Whether this invocation was captured AFTER the case's controller
    /// mutation rather than before it.
    ///
    /// **Criterion 14: a multi-state case is bound per invocation, never to one
    /// case-level default.** The two D02 barrier cases capture three
    /// invocations over TWO states — `baseline` and `barrier` read the state
    /// before the reply writer's append, `clean-rerun` reads the state after
    /// it — and that is the whole point of the case: SC-1306d's snapshot cut is
    /// only visible as the difference between `barrier` (still `pending`, the
    /// reply landed during the scan) and `clean-rerun` (`replied`).
    ///
    /// Binding every invocation to the template would have compared the
    /// `clean-rerun` row against a state it was never captured in, and the
    /// mismatch would have looked like a defect in the sensor.
    fn reads_post_state(&self) -> bool {
        self.consumer.starts_with("clean-rerun")
    }

    /// The container this invocation was captured against.
    ///
    /// For a post-state invocation, the template bytes PLUS the record the
    /// case's own `controller-mutation.txt` says was appended. That file is
    /// frozen evidence — it records the mutation as "append ONE ... reply
    /// event", the payload line verbatim, and the line counts before and after
    /// — so reconstructing the state is reading the corpus, not authoring an
    /// expectation.
    fn container(&self, root: &Path) -> Option<Vec<u8>> {
        let meta = self.meta_dir(root)?;
        let mut body = ae::event_text::read_container(&meta.join(ae::event_text::CONTAINER));
        if !self.reads_post_state() {
            return Some(body);
        }
        let recorded = read_text(
            &root
                .join("batch-c-artifacts")
                .join(&self.case)
                .join("controller-mutation.txt"),
        );
        let payload = recorded
            .lines()
            .find_map(|line| line.strip_prefix("payload_line: "))
            .unwrap_or_else(|| panic!("{}: no recorded payload line", self.label()));
        let before = count_field(&recorded, "events lines BEFORE: ");
        let after = count_field(&recorded, "events lines AFTER: ");
        assert_eq!(
            after,
            before + 1,
            "{}: the recorded mutation must be a single append",
            self.label()
        );
        assert_eq!(
            ae::event_text::read_lines(&body).len(),
            before,
            "{}: the template is not this case's pre-state",
            self.label()
        );
        body.extend_from_slice(payload.as_bytes());
        body.push(b'\n');
        Some(body)
    }
}

/// A `key: <integer>` line from a frozen evidence file.
fn count_field(text: &str, key: &str) -> usize {
    text.lines()
        .find_map(|line| line.strip_prefix(key))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or_else(|| panic!("no {key} field"))
}

/// Every `helper:*` P1 row, with its case metadata resolved.
fn helper_rows(root: &Path) -> Vec<Row> {
    let invocations = read_text(&root.join("corpus").join("INVOCATIONS.tsv"));
    let mut lines = invocations.lines();
    let header: Vec<&str> = lines
        .next()
        .unwrap_or_else(|| panic!("INVOCATIONS.tsv is empty"))
        .split('\t')
        .collect();
    let column = |name: &str| {
        header
            .iter()
            .position(|field| *field == name)
            .unwrap_or_else(|| panic!("INVOCATIONS.tsv has no {name} column"))
    };
    let (case, consumer, rc, phase, surface, argv) = (
        column("case"),
        column("consumer"),
        column("rc"),
        column("phase"),
        column("surface"),
        column("normalised_argv"),
    );

    // case.txt is read once per case, not once per row: several consumers share
    // one case and rereading it would be the only expensive thing here.
    let mut case_meta: BTreeMap<PathBuf, (String, String)> = BTreeMap::new();
    let mut rows = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() <= argv || fields[phase] != "P1" {
            continue;
        }
        if !fields[surface].starts_with("helper:") {
            continue;
        }
        let dir = Path::new(fields[case])
            .parent()
            .unwrap_or_else(|| panic!("{}: no case directory", fields[case]))
            .to_path_buf();
        let (template, session) = case_meta
            .entry(dir.clone())
            .or_insert_with(|| case_fields(root, &dir))
            .clone();
        rows.push(Row {
            case: dir,
            consumer: fields[consumer].to_owned(),
            surface: fields[surface].to_owned(),
            rc: fields[rc]
                .parse()
                .unwrap_or_else(|err| panic!("{}: rc {err}", fields[case])),
            template,
            session,
            argv: fields[argv].to_owned(),
        });
    }
    rows
}

/// `template=` and `session=` from anywhere in a case's `case.txt`.
///
/// The WHOLE file, not its first line: the shape is not uniform across arms.
/// `A1/c01` puts both on line one, `D/d02` puts them on line two, and `A7`/`A9`
/// put each on its own line. A first-line reader silently reported "no template"
/// for 62 rows that have one — which would have quietly shrunk the compared
/// population rather than failing, and is exactly what the asserted
/// unreplayable count exists to catch.
fn case_fields(root: &Path, dir: &Path) -> (String, String) {
    let text = read_text(&root.join("batch-c-artifacts").join(dir).join("case.txt"));
    let field = |key: &str| {
        text.split_whitespace()
            .find_map(|word| word.strip_prefix(key).map(str::to_owned))
            .unwrap_or_default()
    };
    (field("template="), field("session="))
}

/// A readable account of where two byte strings first differ.
fn describe(label: &str, expected: &[u8], actual: &[u8]) -> String {
    let at = expected
        .iter()
        .zip(actual)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    let window = |bytes: &[u8]| {
        let from = at.saturating_sub(24);
        let to = (at + 40).min(bytes.len());
        String::from_utf8_lossy(&bytes[from..to]).into_owned()
    };
    format!(
        "{label}: {} expected bytes vs {} produced, first difference at byte {at}\n  \
         frozen:   {:?}\n  successor: {:?}",
        expected.len(),
        actual.len(),
        window(expected),
        window(actual)
    )
}

#[test]
fn every_frozen_requests_row_matches_byte_for_byte() {
    let root = evidence();
    let rows: Vec<Row> = helper_rows(&root)
        .into_iter()
        .filter(|row| row.surface == "helper:requests")
        .collect();
    assert_eq!(
        rows.len(),
        REQUESTS_ROWS,
        "the pinned helper:requests population changed"
    );

    let mut failures = Vec::new();
    let mut compared = 0;
    let mut unreplayable = Vec::new();
    let mut grep_noise = Vec::new();
    for row in &rows {
        let Some(container) = row.container(&root) else {
            unreplayable.push(row.label());
            continue;
        };
        let mode = row
            .mode()
            .unwrap_or_else(|| panic!("{}: {} is not a mode", row.label(), row.argv));
        // The empty Viewer is what this build has, and it is what every one of
        // these rows was captured with: no tmux server was running for any of
        // them, so the frozen helper detected no identity either.
        let produced = if mode.needs_identity() {
            // The refusal precedes any read, so there is no container to hand
            // over — going through `render` here would need a real directory and
            // would test the same refusal by a longer route.
            requests::Output {
                stdout: Vec::new(),
                stderr: format!("{}\n", requests::NO_IDENTITY).into_bytes(),
                code: requests::EXIT_NO_IDENTITY,
            }
        } else {
            requests::Output {
                stdout: requests::table(&container, mode, &Viewer::default()),
                stderr: Vec::new(),
                code: 0,
            }
        };
        compared += 1;

        let expected_out = row.captured(&root, "stdout");
        if produced.stdout != expected_out {
            failures.push(describe(
                &format!("{} stdout", row.label()),
                &expected_out,
                &produced.stdout,
            ));
        }
        let expected_err = row.captured(&root, "stderr");
        if expected_err.starts_with(b"grep: ") {
            // HELD OUT, NAMED AND COUNTED — never normalised. These bytes are
            // grep's, emitted by the `_lib` bootstrap prologue that every
            // generated helper sources, and they carry the CAPTURE HOST's
            // absolute scratch path. The finding is proved in its own test.
            grep_noise.push(row.label());
        } else if produced.stderr != expected_err {
            failures.push(describe(
                &format!("{} stderr", row.label()),
                &expected_err,
                &produced.stderr,
            ));
        }
        if i32::from(produced.code) != row.rc {
            failures.push(format!(
                "{}: frozen rc {} vs successor {}",
                row.label(),
                row.rc,
                produced.code
            ));
        }
    }

    assert_eq!(
        unreplayable.len(),
        UNREPLAYABLE_ROWS,
        "the set of rows with no fixture bytes changed: {unreplayable:?}"
    );
    assert_eq!(
        grep_noise.len(),
        GREP_NOISE_ROWS,
        "the set of rows whose stderr is grep's changed: {grep_noise:?}"
    );
    assert_eq!(compared, REQUESTS_ROWS - UNREPLAYABLE_ROWS);
    assert!(
        failures.is_empty(),
        "{} of {compared} requests rows differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_meta_bootstrap_grep_noise_is_not_ae_output() {
    // THE SECOND FINDING, AS A TEST. Ten `requests` rows carry stderr that ae
    // never wrote and that no successor can write. Every clause below is
    // asserted rather than asserted-about, so the claim cannot rot:
    //
    //   1. it is `grep(1)`'s diagnostic, not a sentence in ae's vocabulary;
    //   2. it names `<session>/meta` — a file THIS surface never reads, because
    //      the request sensor needs only the event container. The read is the
    //      `_lib` prologue's, which every generated helper sources for the tmux
    //      shim and the session name;
    //   3. it appears TWICE, because that prologue greps `meta` twice without
    //      redirecting stderr (its third grep, for `tmux_server_kind`, does
    //      redirect — which is what makes the count two and not three);
    //   4. it embeds the CAPTURE HOST's absolute scratch path, so the bytes are
    //      unreproducible by construction on any other run;
    //   5. and rc is 0 — the table printed fine. This is leaked noise beside a
    //      successful answer, not a diagnosis of anything.
    //
    // The comparison projection admits diagnostic wording and path detail only
    // through OC-P3-HUMAN-DIAGNOSTIC, whose scope is `human_incomplete_observed`
    // on the HUMAN stderr surface. These are opaque rows. So they stand as a
    // seat question, exactly like the events-tail stderr, and this test exists
    // so the evidence for it is already in the tree.
    let root = evidence();
    let rows: Vec<Row> = helper_rows(&root)
        .into_iter()
        .filter(|row| row.surface == "helper:requests")
        .collect();

    let mut found = Vec::new();
    for row in &rows {
        let captured = row.captured(&root, "stderr");
        if !captured.starts_with(b"grep: ") {
            continue;
        }
        found.push(row.label());
        assert_eq!(row.rc, 0, "{}: the table still printed", row.label());
        assert!(
            !row.captured(&root, "stdout").is_empty(),
            "{}: and its stdout is a real table",
            row.label()
        );
        let text = String::from_utf8(captured).expect("the diagnostic is utf-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "{}: the prologue greps `meta` twice unredirected: {text}",
            row.label()
        );
        assert_eq!(lines[0], lines[1], "{}: the same line twice", row.label());
        assert!(
            lines[0].contains(&format!("/sessions/{}/meta:", row.session)),
            "{}: it names the meta file this surface never reads: {}",
            row.label(),
            lines[0]
        );
        assert!(
            lines[0].contains(CAPTURE_SCRATCH_ROOT),
            "{}: and it embeds the capture host's own scratch path: {}",
            row.label(),
            lines[0]
        );
    }
    assert_eq!(
        found.len(),
        GREP_NOISE_ROWS,
        "the grep-noise population changed: {found:?}"
    );
}

#[test]
fn sc_1306d_the_barrier_cases_show_a_snapshot_cut_and_not_a_stale_read() {
    // The three D02 invocations are one case over two states, and the sensor's
    // snapshot semantics are the DIFFERENCE between them: a reply that landed
    // during the scan leaves the current invocation `pending`, and a clean
    // rerun from the resulting state reports `replied`. Asserting the pair
    // together is what makes it a snapshot claim rather than two unrelated rows.
    let root = evidence();
    let rows: Vec<Row> = helper_rows(&root)
        .into_iter()
        .filter(|row| row.case.starts_with("arms/D"))
        .collect();
    assert_eq!(rows.len(), 5, "the two D02 cases carry five invocations");

    let mut pre = 0;
    let mut post = 0;
    for row in &rows {
        let container = row
            .container(&root)
            .unwrap_or_else(|| panic!("{}: D02 cases are fixture-backed", row.label()));
        let table = requests::table(&container, Mode::All, &Viewer::default());
        let text = String::from_utf8(table).expect("the table is utf-8");
        if row.reads_post_state() {
            post += 1;
            assert!(text.contains("replied  ask"), "{}: {text}", row.label());
            assert!(text.contains("identity-valid answer"), "{}", row.label());
        } else {
            pre += 1;
            assert!(text.contains("pending  ask"), "{}: {text}", row.label());
            assert!(
                !text.contains("identity-valid answer"),
                "{}: the reply is not in this cut",
                row.label()
            );
        }
    }
    assert_eq!((pre, post), (3, 2), "both states are actually exercised");
}

#[test]
fn every_frozen_events_tail_row_matches_byte_for_byte_on_stdout() {
    let root = evidence();
    let rows: Vec<Row> = helper_rows(&root)
        .into_iter()
        .filter(|row| row.surface == "helper:events-tail")
        .collect();
    assert_eq!(
        rows.len(),
        EVENTS_TAIL_ROWS,
        "the pinned helper:events-tail population changed"
    );

    let mut failures = Vec::new();
    for row in &rows {
        let meta = row
            .meta_dir(&root)
            .unwrap_or_else(|| panic!("{}: every events-tail row has a template", row.label()));
        // banner + replay IS the whole of a four-second capture over a container
        // nothing was appending to. Composed here rather than called through
        // `follow`, which by contract never returns.
        let mut produced = events_tail::banner(row.session.as_bytes());
        produced.extend_from_slice(&events_tail::replay(&ae::event_text::read_container(
            &meta.join(ae::event_text::CONTAINER),
        )));

        let expected = row.captured(&root, "stdout");
        if produced != expected {
            failures.push(describe(
                &format!("{} stdout", row.label()),
                &expected,
                &produced,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} events-tail rows differ:\n{}",
        failures.len(),
        rows.len(),
        failures.join("\n")
    );
}

#[test]
fn the_events_tail_stderr_captures_are_bash_artifacts_and_are_not_self_consistent() {
    // THE FINDING, AS A TEST. This does not assert parity — it asserts the two
    // facts that make parity impossible to claim, so the day someone proposes
    // "normalise the stderr and score them green" the evidence is already here
    // and already failing that proposal.
    //
    // 1. Every capture is bash's job-control notification, not ae output.
    // 2. The corpus records TWO different byte strings for it.
    let root = evidence();
    let rows: Vec<Row> = helper_rows(&root)
        .into_iter()
        .filter(|row| row.surface == "helper:events-tail")
        .collect();

    let mut distinct: BTreeMap<Vec<u8>, Vec<String>> = BTreeMap::new();
    for row in &rows {
        assert_eq!(
            row.rc,
            143,
            "{}: every one of these rows is a killed follow",
            row.label()
        );
        let captured = row.captured(&root, "stderr");
        assert!(
            captured.starts_with(b"Terminated: 15"),
            "{}: stderr is expected to be the shell's notification: {:?}",
            row.label(),
            String::from_utf8_lossy(&captured)
        );
        assert!(
            captured.windows(7).any(|window| window == b"tail -n"),
            "{}: and to quote the pipeline it killed",
            row.label()
        );
        distinct.entry(captured).or_default().push(row.label());
    }

    assert_eq!(
        distinct.len(),
        2,
        "the corpus is expected to disagree with itself here; groups: {:?}",
        distinct.values().map(Vec::len).collect::<Vec<usize>>()
    );
    let mut sizes: Vec<usize> = distinct.values().map(Vec::len).collect();
    sizes.sort_unstable();
    assert_eq!(
        sizes,
        vec![1, 37],
        "one row carries a truncated copy of what the other 37 carry"
    );
}

#[test]
fn the_corpus_rows_this_module_covers_are_the_whole_opaque_partition() {
    // The comparison projection fixes the partition at 206 opaque rows, and
    // those 206 are exactly these two surfaces. If a third opaque surface ever
    // appears, this fails rather than leaving it uncovered and unmentioned.
    let rows = helper_rows(&evidence());
    let mut census: BTreeMap<&str, usize> = BTreeMap::new();
    for row in &rows {
        *census.entry(row.surface.as_str()).or_default() += 1;
    }
    assert_eq!(
        census.into_iter().collect::<Vec<(&str, usize)>>(),
        vec![
            ("helper:events-tail", EVENTS_TAIL_ROWS),
            ("helper:requests", REQUESTS_ROWS),
        ]
    );
    assert_eq!(rows.len(), REQUESTS_ROWS + EVENTS_TAIL_ROWS);
}

#[test]
fn the_frozen_argv_of_every_covered_row_maps_to_the_successor_spelling() {
    // Criterion 14 wants the effective normalised argv per invocation. This is
    // the mapping RULE, checked against every frozen argv it claims to cover:
    // the last path component names the helper, the leading component is the
    // meta directory, and a requests row's optional tail is the mode.
    for row in helper_rows(&evidence()) {
        let words: Vec<&str> = row.argv.split_whitespace().collect();
        let script = Path::new(words[0]);
        let helper = script
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let parent = script
            .parent()
            .and_then(Path::to_str)
            .unwrap_or_default()
            .to_owned();
        assert!(
            parent.ends_with(&format!("/sessions/{}", row.session)),
            "{}: {parent} is not this row's session meta directory",
            row.label()
        );
        match row.surface.as_str() {
            "helper:requests" => {
                assert_eq!(helper, "requests", "{}", row.label());
                assert!(words.len() <= 2, "{}: {}", row.label(), row.argv);
                assert!(row.mode().is_some(), "{}: {}", row.label(), row.argv);
            }
            "helper:events-tail" => {
                assert_eq!(helper, "events-tail", "{}", row.label());
                assert_eq!(words.len(), 1, "{}: takes no argument", row.label());
            }
            other => panic!("{}: unexpected surface {other}", row.label()),
        }
    }
}

/// One frozen closure fixture: which routing keys each side carried, and the
/// status the CAPTURE recorded.
struct ClosureShape {
    /// A case directory that runs this fixture (`requests-all`).
    case: &'static str,
    /// A distinguishing fragment of the request id whose row is examined.
    id_fragment: &'static str,
    /// What the frozen capture says that row's status is.
    frozen: &'static str,
    /// Whether SC-518, read strictly, agrees.
    row_agrees: bool,
    /// What the fixture actually contains, for the failure message.
    shape: &'static str,
}

/// THE CLOSURE MATRIX — a ratified row against the captures built to test it.
///
/// **This test asserts a CONFLICT, and the conflict is the deliverable.** It is
/// not a tolerance and it is not a claim that either side is right; it is the
/// evidence, pinned, so a seat can rule on it and so neither side can be lost.
///
/// SC-518 says identity compares as "routing identities (slot+session) when both
/// sides carry them, display identities when neither does, and MIXED identity
/// matches nothing". `src/session.rs::same_participant` implements exactly that,
/// for the `list` attention consumer, and it is right to.
///
/// The frozen `requests` implementation does something different, and the corpus
/// PINS it: it branches on whether the request's target slot and the reply's
/// ACTOR slot are both nonempty, and falls back to display names otherwise. So a
/// request with full routing keys is closed by a keyless reply whose display
/// names mirror it — MIXED identity closing a request. Six fixture shapes below
/// disagree with the row, across twelve `rc=0` corpus rows.
///
/// SC-518's own row records `Empirical: pending (builder P1 implementation +
/// C-cluster)`. The C-cluster is this evidence. It has arrived, and it does not
/// agree with the ruling — which is a finding for the seats that ratified the
/// row, not a thing an implementation may settle by choosing a side.
///
/// Until it is ruled, this surface reproduces the CAPTURES, because byte-identity
/// against the frozen captures is this slice's stated acceptance and a divergence
/// here would be invisible in the one place it is measured.
///
/// # THE SCOPE OF THAT, EXACTLY (reviewer's condition, pubfp 2026-08-24)
///
/// What passing this test proves is FROZEN HELPER PARITY on the ten named
/// shapes. It does NOT prove that the legacy semantics are correct, and it
/// settles nothing about three shapes the corpus never exercises:
///
/// 1. the INVERSE mixed pair — a display-only opening and a routed reply. No
///    fixture has one;
/// 2. a valid reply to an opening that is then RE-ASKED. No fixture re-asks a
///    ref, so nothing pins whether the earlier reply may close the later
///    opening;
/// 3. any `cancel` causality at all. There is not one `cancel` event in the
///    6,862-file corpus.
///
/// This crate's matcher DOES behave some way in all three, because code must:
/// it applies the display fallback symmetrically, and it bounds terminal
/// candidates by identity alone rather than by position. **Neither choice is
/// ratified and neither is captured.** They are the least-surprising extension
/// of the shapes that ARE pinned, chosen so one rule covers both directions
/// rather than being strict one way and loose the other — and they are named
/// here so a seat ruling lands on a decision somebody can see, instead of on a
/// behavior nobody knew was being asserted.
const MATRIX: [ClosureShape; 10] = [
    ClosureShape {
        case: "arms/A7/a7-c12-405j-pair-full-fresh-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        row_agrees: true,
        shape: "both sides fully routed and mirrored",
    },
    ClosureShape {
        case: "arms/A7/a7-c13-405j-pair-stale-keys-ro",
        id_fragment: "c2c01848",
        frozen: "pending",
        row_agrees: true,
        shape: "both sides fully routed, keys do not mirror",
    },
    ClosureShape {
        case: "arms/A7/a7-c14-405j-pair-slot-only-ro",
        id_fragment: "c2c01848",
        frozen: "pending",
        row_agrees: true,
        shape: "reply carries slots but no sessions — half a key matches nothing either way",
    },
    ClosureShape {
        case: "arms/A7/a7-c15-405j-pair-session-only-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        row_agrees: false,
        shape: "reply carries sessions but NO SLOTS; request is fully routed — MIXED",
    },
    ClosureShape {
        case: "arms/A7/a7-c16-405j-pair-keyless-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        row_agrees: false,
        shape: "reply carries no keys at all; request is fully routed — MIXED",
    },
    ClosureShape {
        case: "arms/A7/a7-c17-405j-pair-one-empty-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        row_agrees: false,
        shape: "reply's actor_slot is empty, its other three keys are present — MIXED",
    },
    ClosureShape {
        case: "arms/A7/a7-c18-405j-pair-all-empty-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        row_agrees: false,
        shape: "reply's four keys are all empty strings; request is fully routed — MIXED",
    },
    ClosureShape {
        case: "arms/A6/a6-c06-m6-mixed-routed-display-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        row_agrees: false,
        shape: "the fixture built for this: routed ask, display-only reply — MIXED",
    },
    ClosureShape {
        case: "arms/A1/c14-display-only-legacy-ro",
        id_fragment: "6fb0847b",
        frozen: "replied",
        row_agrees: true,
        shape: "neither side routed — the row's own display fallback",
    },
    ClosureShape {
        case: "arms/A6/a6-c02-m2-wrong-ref-ro",
        id_fragment: "dc302d09",
        frozen: "replied",
        // The SECOND disagreement, and it is about CAUSALITY rather than
        // identity: this reply is a full routed mirror but it sits at line 1
        // and the `review` it closes is at line 3. The frozen sensor's
        // reverse scan retains every candidate and validates by identity
        // alone, so a reply that PRECEDES its own request closes it.
        // `session.rs`'s forward pass cannot: a reply finds nothing open.
        // SC-518's text rules on identity and is silent on ordering.
        row_agrees: false,
        shape: "full routed mirror, but the reply PRECEDES the review it closes",
    },
];

/// The captured status line for one shape, asserted to be what the shape says.
fn frozen_row(root: &Path, shape: &ClosureShape) -> String {
    let captured = std::fs::read(
        root.join("batch-c-artifacts")
            .join(shape.case)
            .join("out")
            .join("requests-all.stdout"),
    )
    .unwrap_or_else(|err| panic!("{}: {err}", shape.case));
    let text = String::from_utf8(captured)
        .unwrap_or_else(|err| panic!("{}: the table is utf-8: {err}", shape.case));
    let row = text
        .lines()
        .find(|line| line.contains(shape.id_fragment))
        .unwrap_or_else(|| panic!("{}: no row for {}", shape.case, shape.id_fragment))
        .to_owned();
    assert!(
        row.starts_with(shape.frozen),
        "{}: the frozen capture says {row:?}, not {}",
        shape.case,
        shape.frozen
    );
    row
}

/// This surface's own table over that shape's fixture bytes.
fn produced_table(root: &Path, shape: &ClosureShape) -> String {
    let (template, session) = case_fields(root, Path::new(shape.case));
    let (group, member) = template
        .split_once('/')
        .unwrap_or_else(|| panic!("{}: template {template:?}", shape.case));
    let container = ae::event_text::read_container(
        &root
            .join("batch-c-artifacts")
            .join("templates")
            .join(group)
            .join("fixture-bytes")
            .join(member)
            .join("sessions")
            .join(&session)
            .join(ae::event_text::CONTAINER),
    );
    String::from_utf8(requests::table(&container, Mode::All, &Viewer::default()))
        .unwrap_or_else(|err| panic!("{}: the table is utf-8: {err}", shape.case))
}

#[test]
fn sc_518_the_frozen_closure_matrix_disagrees_with_the_row_on_mixed_identity() {
    let root = evidence();
    let mut disagreements = Vec::new();
    for shape in &MATRIX {
        let row = frozen_row(&root, shape);
        // And THIS surface reproduces that capture — the 168-row comparison
        // covers it, but naming the specific rows here is what makes the
        // conflict legible instead of buried in a population count.
        let produced = produced_table(&root, shape);
        assert!(
            produced.lines().any(|line| line == row),
            "{}: this surface does not reproduce {row:?}\n{produced}",
            shape.case
        );
        if !shape.row_agrees {
            disagreements.push(format!("  {} — {}", shape.case, shape.shape));
        }
    }

    assert_eq!(
        disagreements.len(),
        6,
        "the set of fixtures where SC-518 as written and the frozen captures \
         disagree changed:\n{}",
        disagreements.join("\n")
    );
}
