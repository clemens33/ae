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

/// The rows the SC-518 / SC-518a ruling MOVES, and nothing else.
///
/// **A MANDATED DIVERGENCE IS ASSERTED AS PRECISELY AS PARITY IS.** These twelve
/// rows must differ from their captures, must differ in exactly the ruled way
/// (the status token and the summary, never an identity column), and must be the
/// ONLY rows that differ. A thirteenth fails; a twelfth that stops differing
/// fails too, because that is the ruling silently coming un-applied.
///
/// Twelve rows and not twenty: the pinned matrix has ten shapes with byte-
/// identical `-ro`/`-rw` twins, but only SIX shapes move, so twelve rows change
/// bytes. Both numbers are true of different things and the distinction is
/// recorded here because a reader checking twenty against a diff finds twelve.
const RULED_DIVERGENCE: [&str; 12] = [
    "arms/A6/a6-c02-m2-wrong-ref-ro",
    "arms/A6/a6-c02-m2-wrong-ref-rw",
    "arms/A6/a6-c06-m6-mixed-routed-display-ro",
    "arms/A6/a6-c06-m6-mixed-routed-display-rw",
    "arms/A7/a7-c15-405j-pair-session-only-ro",
    "arms/A7/a7-c15-405j-pair-session-only-rw",
    "arms/A7/a7-c16-405j-pair-keyless-ro",
    "arms/A7/a7-c16-405j-pair-keyless-rw",
    "arms/A7/a7-c17-405j-pair-one-empty-ro",
    "arms/A7/a7-c17-405j-pair-one-empty-rw",
    "arms/A7/a7-c18-405j-pair-all-empty-ro",
    "arms/A7/a7-c18-405j-pair-all-empty-rw",
];

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

/// What the stdout comparison found for one row.
enum Stdout {
    /// Byte-identical to its capture, as 153 of the rows must be.
    Matched,
    /// Differs, and differs in exactly the way the ruling requires.
    Diverged,
    /// Differs in a way nothing authorises. Carries the account of it.
    Wrong(String),
}

/// Compare one row's stdout, holding the RULED rows to the ruled DIFFERENCE and
/// every other row to byte identity.
fn check_stdout(row: &Row, expected: &[u8], produced: &[u8]) -> Stdout {
    let ruled = RULED_DIVERGENCE.contains(&row.case.to_string_lossy().as_ref())
        && row.consumer == "requests-all";
    if !ruled {
        return if produced == expected {
            Stdout::Matched
        } else {
            Stdout::Wrong(describe(
                &format!("{} stdout", row.label()),
                expected,
                produced,
            ))
        };
    }
    // A ruled row that still matches its capture is the ruling silently coming
    // un-applied, which is a failure and not a pass.
    if produced == expected {
        return Stdout::Wrong(format!(
            "{}: the ruling was NOT applied — this row still matches its pre-ruling capture",
            row.label()
        ));
    }
    match ruled_flip(expected, produced) {
        Some(wrong) => Stdout::Wrong(format!("{}: {wrong}", row.label())),
        None => Stdout::Diverged,
    }
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
    let mut diverged = Vec::new();
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
        match check_stdout(row, &expected_out, &produced.stdout) {
            Stdout::Matched => {}
            Stdout::Diverged => diverged.push(row.label()),
            Stdout::Wrong(why) => failures.push(why),
        }

        let expected_err = row.captured(&root, "stderr");
        if expected_err.starts_with(b"grep: ") {
            // NAMED AND COUNTED — never normalised. These bytes are grep's,
            // emitted by the `_lib` bootstrap prologue every generated helper
            // sources, and they carry the CAPTURE HOST's absolute scratch path.
            // The finding is proved in its own test.
            //
            // What is asserted here is the SUCCESSOR side, and it is asserted
            // rather than skipped: this surface writes EXACTLY NOTHING to
            // stderr on a successful table. So the successor half of whatever
            // subtraction the seats ratify is already true and already checked,
            // and "held out" never means "unknown".
            //
            // FOLD CONDITION — when the member-4 stderr ruling lands, the
            // BASELINE side changes shape and this branch must be re-keyed:
            // scope comes from the ruling's fixed pre-successor JOINS
            // (baseline-helper-bootstrap-grep), never from a byte prefix. The
            // `starts_with(b"grep: ")` test above is a tolerance wearing a
            // join's clothes and must be DELETED at that point — colead's seed
            // list plants grep-shaped stderr OUTSIDE the keyed loci precisely to
            // catch a prefix key, and a prefix key swallows it. The assertion
            // below survives the change unaltered.
            assert!(
                produced.stderr.is_empty(),
                "{}: the successor must write no stderr beside a successful \
                 table, got {:?}",
                row.label(),
                String::from_utf8_lossy(&produced.stderr)
            );
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
    diverged.sort();
    let mut expected_divergence: Vec<String> = RULED_DIVERGENCE
        .iter()
        .map(|case| format!("{case} :: requests-all"))
        .collect();
    expected_divergence.sort();
    assert_eq!(
        diverged, expected_divergence,
        "the set of rows the ruling moves changed"
    );
    assert_eq!(compared, REQUESTS_ROWS - UNREPLAYABLE_ROWS);
    assert!(
        failures.is_empty(),
        "{} of {compared} requests rows differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// `None` when `produced` differs from `frozen` in exactly the way the ruling
/// requires, or the reason it does not.
///
/// The ruled difference is narrow and worth checking rather than assuming: same
/// number of lines, the header untouched, and every changed line changed only in
/// its STATUS token and its SUMMARY — `replied` becoming `pending`, with the
/// identity columns byte-identical. Anything else is a defect wearing a
/// ruling's clothes.
fn ruled_flip(frozen: &[u8], produced: &[u8]) -> Option<String> {
    let (frozen, produced) = (
        String::from_utf8_lossy(frozen),
        String::from_utf8_lossy(produced),
    );
    let (before, after): (Vec<&str>, Vec<&str>) =
        (frozen.lines().collect(), produced.lines().collect());
    if before.len() != after.len() {
        return Some(format!(
            "row count moved: {} -> {}",
            before.len(),
            after.len()
        ));
    }
    let mut flips = 0;
    for (index, (was, now)) in before.iter().zip(&after).enumerate() {
        if was == now {
            continue;
        }
        if index == 0 {
            return Some("the header changed".to_owned());
        }
        if fields(was)[1..5] != fields(now)[1..5] {
            return Some(format!("an identity column moved: {was:?} -> {now:?}"));
        }
        let ((was_status, _), (now_status, _)) = (status_and_summary(was), status_and_summary(now));
        if (was_status, now_status) != ("replied", "pending") {
            return Some(format!(
                "the status moved {was_status} -> {now_status}, not replied -> pending"
            ));
        }
        flips += 1;
    }
    if flips == 0 {
        return Some("nothing actually changed".to_owned());
    }
    None
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

/// One pinned closure fixture: what the CAPTURE says, and what the RULING says.
struct ClosureShape {
    /// A case directory that runs this fixture (`requests-all`), `-ro` half.
    case: &'static str,
    /// A distinguishing fragment of the request id whose row is examined.
    id_fragment: &'static str,
    /// The status the frozen capture recorded.
    frozen: &'static str,
    /// The status the RULED matcher must produce.
    ruled: &'static str,
    /// The summary the RULED matcher must produce.
    ///
    /// This is the consequence that is easy to miss: a row moving from
    /// `replied` back to `pending` loses the reply's text and shows the
    /// OPENING's again. The frozen capture already takes FROM/TO from the
    /// opening and only the summary from the terminal — visible directly in the
    /// m2 capture, whose row reads `fake:lead -> fake:worker` (the review's
    /// participants) beside `G5 mirror answer` (the reply's text) — which is why
    /// the identity columns do not move and this one does.
    ruled_summary: &'static str,
    /// Why this shape lands where it lands.
    shape: &'static str,
}

const MATRIX: [ClosureShape; 10] = [
    ClosureShape {
        case: "arms/A7/a7-c12-405j-pair-full-fresh-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        ruled: "replied",
        ruled_summary: "G5 mirror answer",
        shape: "routed to routed, mirrored — the only closing shape left besides display to display",
    },
    ClosureShape {
        case: "arms/A7/a7-c13-405j-pair-stale-keys-ro",
        id_fragment: "c2c01848",
        frozen: "pending",
        ruled: "pending",
        ruled_summary: "G5 mirror question",
        shape: "routed to routed, naming a different slot AND session",
    },
    ClosureShape {
        case: "arms/A7/a7-c14-405j-pair-slot-only-ro",
        id_fragment: "c2c01848",
        frozen: "pending",
        ruled: "pending",
        ruled_summary: "G5 mirror question",
        shape: "reply carries slots and no sessions — Unassociated, matches nothing",
    },
    ClosureShape {
        case: "arms/A7/a7-c15-405j-pair-session-only-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        ruled: "pending",
        ruled_summary: "G5 mirror question",
        shape: "reply carries sessions and no slots — Unassociated. Frozen never entered its \
                routed branch (no actor_slot) and fell back to names, which is why this one \
                moves while slot-only does not",
    },
    ClosureShape {
        case: "arms/A7/a7-c16-405j-pair-keyless-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        ruled: "pending",
        ruled_summary: "G5 mirror question",
        shape: "reply carries no keys — Display against a Routed request, the MIXED pair",
    },
    ClosureShape {
        case: "arms/A7/a7-c17-405j-pair-one-empty-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        ruled: "pending",
        ruled_summary: "G5 mirror question",
        shape: "reply's actor_slot is present and EMPTY, its other three real — Unassociated, \
                and NOT the same thing as absent",
    },
    ClosureShape {
        case: "arms/A7/a7-c18-405j-pair-all-empty-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        ruled: "pending",
        ruled_summary: "G5 mirror question",
        shape: "reply's four keys all present and EMPTY — Unassociated, which matches nothing \
                including another Unassociated",
    },
    ClosureShape {
        case: "arms/A6/a6-c06-m6-mixed-routed-display-ro",
        id_fragment: "c2c01848",
        frozen: "replied",
        ruled: "pending",
        ruled_summary: "G5 mirror question",
        shape: "the fixture built for this: routed ask, display-only reply",
    },
    ClosureShape {
        case: "arms/A1/c14-display-only-legacy-ro",
        id_fragment: "6fb0847b",
        frozen: "replied",
        ruled: "replied",
        ruled_summary: "healthy fixture answer",
        shape: "ZERO routing keys on BOTH events — the only display-to-display specimen there is",
    },
    ClosureShape {
        case: "arms/A6/a6-c02-m2-wrong-ref-ro",
        id_fragment: "dc302d09",
        frozen: "replied",
        ruled: "pending",
        ruled_summary: "G5 review request (second well-formed ref donor)",
        shape: "SC-518a, not identity: a full routed mirror that PRECEDES the review it closes",
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

/// THE CLOSURE MATRIX — the ruled semantics against the captures they replace.
///
/// **SC-518 WAS RULED STRICT (2026-08-24) and SC-518a was born beside it.** The
/// safety direction decided it: a false PENDING is loud and costs a human a
/// second glance, while a false CLOSURE silently erases a real request. So
/// frozen's mixed-identity closure is a DEFECT, and so is its pre-opening
/// closure — this surface DIVERGES from six of the ten captures below, and that
/// divergence is MANDATED rather than tolerated.
///
/// This test is the ledger of it. Each shape carries what the capture recorded
/// AND what the ruling requires, so:
///
/// * a shape that stops diverging fails — the ruling would have been silently
///   un-applied;
/// * a shape that starts diverging fails — parity would have been silently lost
///   somewhere the ruling does not reach;
/// * and `ruled_summary` pins the consequence easiest to miss: a row falling
///   back to `pending` shows the OPENING's text again, not the reply's.
///
/// The ruled values were derived TWICE, independently — once from the fixture
/// bytes by a script sharing no code with the matcher, and once by `opus5:lexec`
/// from the same bytes before either derivation was shared. They agreed on all
/// ten statuses and all ten summaries. That agreement is why these are
/// hard-coded values and not something computed from the thing under test.
#[test]
fn sc_518_the_ruled_closure_matrix_replaces_six_of_the_frozen_captures() {
    let root = evidence();
    let mut moved = Vec::new();
    for shape in &MATRIX {
        let frozen = frozen_row(&root, shape);
        let produced = produced_table(&root, shape);
        let ruled = produced
            .lines()
            .find(|line| line.contains(shape.id_fragment))
            .unwrap_or_else(|| panic!("{}: no ruled row for {}", shape.case, shape.id_fragment));

        let (status, summary) = status_and_summary(ruled);
        assert_eq!(status, shape.ruled, "{}: {}", shape.case, shape.shape);
        assert_eq!(
            summary, shape.ruled_summary,
            "{}: the summary follows the status back to the opening",
            shape.case
        );

        // The identity columns never move. Frozen already took FROM/TO from the
        // OPENING and only the summary from the terminal — visible in the m2
        // capture itself, whose row names the review's participants beside the
        // reply's text — so a status falling back carries the summary and
        // nothing else with it.
        assert_eq!(
            fields(ruled)[1..5],
            fields(&frozen)[1..5],
            "{}: kind/id/from/to must be identical",
            shape.case
        );
        let (frozen_status, _) = status_and_summary(&frozen);
        assert_eq!(frozen_status, shape.frozen, "{}", shape.case);
        if frozen_status != shape.ruled {
            moved.push(format!("  {} — {}", shape.case, shape.shape));
        }
    }
    assert_eq!(
        moved.len(),
        6,
        "the set of shapes the ruling moves changed:\n{}",
        moved.join("\n")
    );
}

/// The five fixed columns of a table line, as whitespace-separated tokens.
///
/// Split on whitespace rather than at column offsets because a column can
/// OVERFLOW — the A6 fixture carries a 31-character request id in a 28-wide
/// column. None of the five can contain a space, so this is exact.
fn fields(line: &str) -> [String; 5] {
    let mut words = line.split_whitespace();
    core::array::from_fn(|_| words.next().unwrap_or_default().to_owned())
}

/// A table line's status token and its summary remainder.
///
/// The summary is whatever follows the five fixed columns, taken as a
/// REMAINDER rather than as a token: it contains spaces, and one of these
/// fixtures ends its summary in a parenthesis.
fn status_and_summary(line: &str) -> (&str, &str) {
    let mut rest = line;
    let mut status = "";
    for index in 0..5 {
        let trimmed = rest.trim_start();
        let end = trimmed.find(' ').unwrap_or(trimmed.len());
        if index == 0 {
            status = &trimmed[..end];
        }
        rest = &trimmed[end..];
    }
    (status, rest.trim_start())
}

/// THE CROSS-CONSUMER AGREEMENT — one container, both readers, same verdict.
///
/// `Identity::matches` is now shared (`events.rs`, landed by `opus5:reason2`), so
/// the two consumers can no longer disagree about what an identity IS. They can
/// still disagree about how they USE it, and that is the interesting half:
/// `session.rs::pending_requests` walks a FORWARD pass and retains-on-close,
/// while `requests::states` walks a REVERSED pass and picks a winner per ref.
/// Two different algorithms over one rule is exactly the shape that drifts.
///
/// **This is the mechanism a shared function is not.** It compares the two
/// CONSUMERS on containers that include the shapes the SC-518 ruling FLIPPED —
/// per `reason2`'s design note, because a flip changes which record supplies a
/// row's summary, so a flipped shape is a second place to disagree beyond the
/// closure verdict itself. Both facts are asserted on the same bytes.
///
/// A disagreement here is a finding against WHICHEVER reader is wrong, and the
/// two of us agreed in advance that it is not presumed to be either one.
#[test]
fn the_two_request_readers_agree_on_closure_and_on_the_summary_source() {
    use ae::events::{Cursor, EventLog};
    use ae::requests::Status;

    // Every shape that matters, in one container: a closing routed pair, a
    // MIXED pair the ruling flipped, an inverse-mixed pair with no corpus
    // specimen, a keyless closing pair, a pre-opening terminal, and a re-ask.
    const CONTAINER_LINES: [&str; 11] = [
        // r1 — routed mirror, CLOSES.
        r#"{"ts":"2026-08-20T16:00:01Z","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"r1 asked"}"#,
        r#"{"ts":"2026-08-20T16:00:02Z","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"worker.0","actor_session":"s","target_slot":"main","target_session":"s","summary":"r1 answered"}"#,
        // r2 — routed ask, keyless reply: the MIXED shape the ruling flipped.
        r#"{"ts":"2026-08-20T16:00:03Z","actor":"a:lead","action":"ask","target":"a:worker","ref":"r2","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"r2 asked"}"#,
        r#"{"ts":"2026-08-20T16:00:04Z","actor":"a:worker","action":"reply","target":"a:lead","ref":"r2","summary":"r2 answered"}"#,
        // r3 — keyless ask, routed reply: the INVERSE mixed shape.
        r#"{"ts":"2026-08-20T16:00:05Z","actor":"a:lead","action":"ask","target":"a:worker","ref":"r3","summary":"r3 asked"}"#,
        r#"{"ts":"2026-08-20T16:00:06Z","actor":"a:worker","action":"reply","target":"a:lead","ref":"r3","actor_slot":"worker.0","actor_session":"s","target_slot":"main","target_session":"s","summary":"r3 answered"}"#,
        // r4 — keyless on both sides: display to display, CLOSES.
        r#"{"ts":"2026-08-20T16:00:07Z","actor":"a:lead","action":"ask","target":"a:worker","ref":"r4","summary":"r4 asked"}"#,
        r#"{"ts":"2026-08-20T16:00:08Z","actor":"a:worker","action":"reply","target":"a:lead","ref":"r4","summary":"r4 answered"}"#,
        // r5 — the terminal PRECEDES its opening (SC-518a).
        r#"{"ts":"2026-08-20T16:00:09Z","actor":"a:worker","action":"reply","target":"a:lead","ref":"r5","summary":"r5 answered early"}"#,
        r#"{"ts":"2026-08-20T16:00:10Z","actor":"a:lead","action":"ask","target":"a:worker","ref":"r5","summary":"r5 asked"}"#,
        // r6 — asked, answered, then RE-ASKED: a new lifecycle.
        r#"{"ts":"2026-08-20T16:00:11Z","actor":"a:lead","action":"ask","target":"a:worker","ref":"r6","summary":"r6 asked"}"#,
    ];
    // r6's reply and re-ask are appended below so the line count stays readable.
    let mut body = String::new();
    for line in CONTAINER_LINES {
        body.push_str(line);
        body.push('\n');
    }
    body.push_str(
        r#"{"ts":"2026-08-20T16:00:12Z","actor":"a:worker","action":"reply","target":"a:lead","ref":"r6","summary":"r6 answered"}"#,
    );
    body.push('\n');
    body.push_str(
        r#"{"ts":"2026-08-20T16:00:13Z","actor":"a:lead","action":"ask","target":"a:worker","ref":"r6","summary":"r6 re-asked"}"#,
    );
    body.push('\n');

    // Which refs each reader considers OPEN.
    let table_rows = requests::states(body.as_bytes());
    let mut open_per_table: Vec<&str> = table_rows
        .iter()
        .filter(|row| row.status != Status::Replied && row.status != Status::Cancelled)
        .map(|row| std::str::from_utf8(&row.id).expect("ascii ids"))
        .collect();
    open_per_table.sort_unstable();

    let scratch = std::env::temp_dir().join(format!("ae-crossreader-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("scratch");
    std::fs::write(scratch.join(ae::event_text::CONTAINER), &body).expect("write");
    let drain = EventLog::discover(&scratch)
        .drain(Cursor::default())
        .expect("the container reads");
    let read = ae::session::SessionRead::from_drain(&drain);
    let mut open_per_session: Vec<&str> = read
        .pending
        .iter()
        .map(|request| request.id.as_str())
        .collect();
    open_per_session.sort_unstable();
    let _ = std::fs::remove_dir_all(&scratch);

    assert_eq!(
        open_per_table, open_per_session,
        "the two readers disagree about which requests are open"
    );
    // And the set is what the RULING says it is, so agreement on a wrong answer
    // cannot pass: r1 and r4 close (routed mirror, display mirror), r2 and r3
    // do not (both mixed, one in each direction), r5 does not (its terminal
    // precedes it), r6 does not (its terminal belongs to the old lifecycle).
    assert_eq!(open_per_table, vec!["r2", "r3", "r5", "r6"]);

    // THE SUMMARY SOURCE, on the same bytes — reason2's second place to
    // disagree. A closed row shows the TERMINAL's text; an open row shows the
    // OPENING's, including the two the ruling flipped back.
    let summary_of = |id: &str| -> String {
        table_rows
            .iter()
            .find(|row| row.id == id.as_bytes())
            .map_or_else(
                || panic!("no row for {id}"),
                |row| String::from_utf8_lossy(&row.summary).into_owned(),
            )
    };
    assert_eq!(
        summary_of("r1"),
        "r1 answered",
        "closed: the terminal's text"
    );
    assert_eq!(summary_of("r4"), "r4 answered");
    assert_eq!(
        summary_of("r2"),
        "r2 asked",
        "flipped back: the opening's text"
    );
    assert_eq!(summary_of("r3"), "r3 asked");
    assert_eq!(summary_of("r5"), "r5 asked");
    assert_eq!(
        summary_of("r6"),
        "r6 re-asked",
        "the NEW lifecycle's opening"
    );
}
