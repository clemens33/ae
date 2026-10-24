//! Frozen acceptance contract: a bounded history of vendor quota samples and
//! seat workload (#243 P1, lead rulings R1-R7, plan r3 API).
//!
//! Oracle: #243 and the brief's rulings, never the implementation. The wire
//! keys are the plan's struct field names; an absent optional and `null` are
//! the same fact. RED tests fail on base at an assertion; CONTROL tests pin
//! unchanged behaviour and pass on base and after (named in the receipt).

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect process-owned scratch state roots only"
)]

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ae::quota::history::{
    self, ALIGN_SECS, PolicyFacts, Report, Row, Sample, Scope, Seat, Skipped, Workload,
};

/// 2033-05-18T11:53:20Z: midday, so +-300 s never crosses a UTC date.
const NOW: i64 = 2_000_030_000;
const DAY: i64 = 86_400;
const RESET: i64 = NOW + 3 * DAY;
const INC: [&str; 7] = [
    "11111111-1111-4111-8111-111111111111",
    "22222222-2222-4222-8222-222222222222",
    "33333333-3333-4333-8333-333333333333",
    "44444444-4444-4444-8444-444444444444",
    "55555555-5555-4555-8555-555555555555",
    "66666666-6666-4666-8666-666666666666",
    "77777777-7777-4777-8777-777777777777",
];
const CAP: u64 = 16 * 1024 * 1024;
const WAIT: Duration = Duration::from_mins(1);

/// `YYYY-MM-DD` of an epoch, UTC — an independent civil-date oracle.
fn date(epoch: i64) -> String {
    let z = epoch.div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[test]
fn the_date_oracle_is_right() {
    assert_eq!(date(NOW), "2033-05-18");
    assert_eq!(date(0), "1970-01-01");
    assert_eq!(date(951_782_400), "2000-02-29");
}

fn root(tag: &str) -> PathBuf {
    super::cli::OwnedScratch::root("qh", tag).keep()
}

/// A session dir under `root` whose meta names `incarnation`.
fn session(root: &Path, name: &str, incarnation: Option<&str>) -> PathBuf {
    let dir = root.join("sessions").join(name);
    std::fs::create_dir_all(&dir).expect("session dir");
    let id = incarnation.map_or_else(String::new, |id| format!("session_id={id}\n"));
    std::fs::write(dir.join("meta"), format!("session={name}\n{id}")).expect("meta");
    dir
}

fn policy(resets: Option<u8>) -> PolicyFacts {
    PolicyFacts {
        version: 1,
        manual_resets: resets,
        credits: "unreported".to_owned(),
        spend_cap: None,
    }
}

fn scope(home: &str) -> Scope {
    Scope {
        tool: "claude".to_owned(),
        source: format!("{home}/.claude.json"),
        account: Some(home.to_owned()),
    }
}

fn sample(producer: &str, inc: &str, observed_at: i64, raw: &str) -> Sample {
    let effective = format!("{:.6}", raw.parse::<f64>().expect("fixture raw"));
    Sample {
        producer: producer.to_owned(),
        incarnation: inc.to_owned(),
        collected_at: observed_at + 10,
        scope: scope("/h"),
        bucket: "weekly_all".to_owned(),
        qualifier: None,
        window_minutes: Some(10_080),
        resets_at: RESET,
        observed_at,
        raw: raw.to_owned(),
        effective,
        derivation: "raw".to_owned(),
        policy: policy(None),
    }
}

fn workload(producer: &str, inc: &str, collected_at: i64, observed_at: Option<i64>) -> Workload {
    Workload {
        producer: producer.to_owned(),
        incarnation: inc.to_owned(),
        collected_at,
        scope: scope("/h"),
        observed_at,
        seats: vec![Seat {
            slot: "main".to_owned(),
            agent: "lead".to_owned(),
            launch: Some(INC[6].to_owned()),
            observed: "busy".to_owned(),
            declared: Some("working".to_owned()),
            model: Some("opus".to_owned()),
            effort: Some("high".to_owned()),
        }],
        unscoped: 1,
    }
}

/// Each row rendered, newline-terminated, in order.
fn lines_of(rows: &[Row]) -> String {
    rows.iter().fold(String::new(), |mut out, row| {
        let _ = writeln!(out, "{}", history::render(row));
        out
    })
}

/// Write rendered rows as one day file, bypassing `append`, for reader pins.
fn plant(dir: &Path, day: i64, rows: &[Row], tail: &str) -> PathBuf {
    let history = dir.join("quota-history");
    std::fs::create_dir_all(&history).expect("history dir");
    private(&history);
    let file = history.join(format!("{}.jsonl", date(day)));
    let mut text = lines_of(rows);
    text.push_str(tail);
    std::fs::write(&file, text).expect("planted day file");
    private(&file);
    file
}

/// The modes the writer itself would use: 0700 for a dir, 0600 for a file.
fn private(path: &Path) {
    let bits = if path.is_dir() { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).expect("private mode");
}

/// The permission bits of a node, `None` when there is no node.
fn mode(path: &Path) -> Option<u32> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o777)
}

fn jsonl(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir.join("quota-history")).map_or_else(
        |_| Vec::new(),
        |entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| std::fs::DirEntry::path(&entry))
                .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
                .collect()
        },
    )
}

fn at(report: &Report, observed_at: i64) -> usize {
    report
        .samples
        .iter()
        .position(|sample| sample.observed_at == observed_at)
        .unwrap_or_else(|| panic!("no sample observed at {observed_at}: {report:?}"))
}

/// Every segment as (`observed_at` of its sample, reason, correction), sorted.
fn segments(report: &Report) -> Vec<(i64, String, bool)> {
    let mut out: Vec<(i64, String, bool)> = report
        .segments
        .iter()
        .map(|seg| {
            (
                report.samples[seg.sample].observed_at,
                seg.reason.clone(),
                seg.correction,
            )
        })
        .collect();
    out.sort();
    out
}

const MIB: usize = 1024 * 1024;

/// A `u64` size as `usize`; every fixture size fits.
fn n(bytes: u64) -> usize {
    usize::try_from(bytes).expect("fixture size fits usize")
}

/// The n-th canonical session id past `INC`, for fleets wider than seven.
fn uuid(k: usize) -> String {
    format!("{:08x}-aaaa-4aaa-8aaa-{k:012x}", k + 1)
}

/// The bytes a row takes on disk: its line and its newline.
fn len_of(row: &Row) -> usize {
    history::render(row).len() + 1
}

/// A valid workload row of 128 distinct seats, about 51 KB rendered.
fn heavy(producer: &str, inc: &str, collected_at: i64) -> Workload {
    let mut row = workload(producer, inc, collected_at, Some(collected_at - 5));
    let seat = row.seats[0].clone();
    row.seats = (0..128)
        .map(|k| Seat {
            slot: format!("s{k:03}"),
            agent: format!("a{k:03}{}", "x".repeat(250)),
            ..seat.clone()
        })
        .collect();
    row
}

/// Valid rows of one session whose rendered lines total exactly `bytes`:
/// heavy workload lines, then samples whose qualifier length takes the rest.
fn filler(producer: &str, inc: &str, bytes: usize) -> String {
    let big = history::render(&Row::Workload(heavy(producer, inc, NOW - 100)));
    let mut probe = sample(producer, inc, NOW - 60, "41");
    probe.qualifier = Some("q".to_owned());
    let small = len_of(&Row::Sample(probe.clone()));
    assert!(
        big.len() > 40_000 && small > 100,
        "fixture rows render: {} / {small}",
        big.len()
    );
    let line = big.len() + 1;
    let count = bytes.saturating_sub(4096) / line;
    let mut text = format!("{big}\n").repeat(count);
    let rest = bytes - count * line;
    let lines = rest.div_ceil(small + 255);
    assert!(lines * small <= rest, "filler arithmetic");
    let mut extra = rest - lines * small;
    for _ in 0..lines {
        let add = extra.min(255);
        extra -= add;
        probe.qualifier = Some("q".repeat(1 + add));
        text.push_str(&history::render(&Row::Sample(probe.clone())));
        text.push('\n');
    }
    assert_eq!(text.len(), bytes, "the filler is exact");
    text
}

/// Junk regular files with names that are not dates, to fill a history dir.
fn junk(dir: &Path, count: usize) {
    for k in 0..count {
        let path = dir.join("quota-history").join(format!("note-{k:03}.txt"));
        std::fs::write(&path, "x\n").expect("junk entry");
        private(&path);
    }
}

/// Removes a heavy fixture root when its test ends, pass or fail.
struct Gone(PathBuf);

impl Drop for Gone {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------- grammar

#[test]
fn a_record_renders_as_one_versioned_line_and_parses_back_to_itself() {
    let mut bare = workload("s", INC[0], NOW, None);
    bare.seats[0] = Seat {
        launch: None,
        declared: None,
        model: None,
        effort: None,
        ..bare.seats[0].clone()
    };
    let mut odd = sample("s", INC[0], NOW - 60, "41.5");
    odd.effective = "13.833333".to_owned();
    odd.derivation = "resets".to_owned();
    odd.qualifier = Some("opus".to_owned());
    odd.window_minutes = None;
    odd.policy = PolicyFacts {
        version: 1,
        manual_resets: Some(2),
        credits: "available-unknown".to_owned(),
        spend_cap: Some(false),
    };
    for row in [
        Row::Sample(sample("s", INC[0], NOW - 60, "41")),
        Row::Sample(odd),
        Row::Workload(workload("s", INC[0], NOW, Some(NOW - 5))),
        Row::Workload(bare),
    ] {
        let line = history::render(&row);
        assert!(
            !line.is_empty() && !line.contains('\n'),
            "one line: {line:?}"
        );
        let value = ae::json::parse(&line).expect("the line is one JSON object");
        assert_eq!(
            value.get("v").map(ae::json::Value::render).as_deref(),
            Some("1")
        );
        let kind = if matches!(row, Row::Sample(_)) {
            "sample"
        } else {
            "workload"
        };
        assert_eq!(value.get_str("kind"), Some(kind), "{line}");
        assert_eq!(history::parse(&line), Ok(row), "{line}");
    }
}

const WIRE: &str = r#"{"unknown_future_key":[1,2],"kind":"sample","v":1,"incarnation":"11111111-1111-4111-8111-111111111111","producer":"s","collected_at":2000029950,"scope":{"source":"/h/.claude.json","tool":"claude","account":"/h"},"bucket":"weekly_all","window_minutes":10080,"resets_at":2000289200,"observed_at":2000029940,"raw":"41","effective":"41.000000","derivation":"raw","policy":{"credits":"unreported","version":1,"manual_resets":null}}"#;

#[test]
fn the_wire_names_are_the_plan_fields_in_any_order_with_unknown_keys_inert() {
    let mut want = sample("s", INC[0], NOW - 60, "41");
    want.collected_at = NOW - 50;
    assert_eq!(history::parse(WIRE), Ok(Row::Sample(want)));
    let workload_line = r#"{"v":1,"kind":"workload","producer":"s","incarnation":"11111111-1111-4111-8111-111111111111","collected_at":2000030000,"scope":{"tool":"claude","source":"/h/.claude.json","account":null},"observed_at":null,"unscoped":0,"seats":[{"slot":"main","agent":"lead","observed":"unknown"}]}"#;
    let Ok(Row::Workload(row)) = history::parse(workload_line) else {
        panic!(
            "a minimal workload line parses: {:?}",
            history::parse(workload_line)
        );
    };
    assert_eq!(
        (row.observed_at, row.unscoped, row.scope.account),
        (None, 0, None)
    );
    assert_eq!(row.seats[0].observed, "unknown");
    assert_eq!(row.seats[0].declared, None);
}

#[test]
fn a_hostile_line_is_refused_for_each_single_cause() {
    assert!(
        history::parse(WIRE).is_ok(),
        "control: the unmutated line parses"
    );
    let long = format!("\"{}\"", "p".repeat(257));
    let raw33 = format!("\"{}\"", "1".repeat(33));
    let pad = "x".repeat(65_536);
    let refused = [
        ("missing producer", WIRE.replace(r#""producer":"s","#, "")),
        (
            "duplicate key",
            WIRE.replace(
                r#""bucket":"weekly_all","#,
                r#""bucket":"weekly_all","bucket":"session","#,
            ),
        ),
        ("version", WIRE.replace(r#""v":1"#, r#""v":2"#)),
        (
            "kind",
            WIRE.replace(r#""kind":"sample""#, r#""kind":"other""#),
        ),
        (
            "type",
            WIRE.replace(r#""resets_at":2000289200"#, r#""resets_at":"2000289200""#),
        ),
        (
            "zero epoch",
            WIRE.replace(r#""resets_at":2000289200"#, r#""resets_at":0"#),
        ),
        (
            "zero window",
            WIRE.replace(r#""window_minutes":10080"#, r#""window_minutes":0"#),
        ),
        (
            "negative raw",
            WIRE.replace(r#""raw":"41""#, r#""raw":"-1""#),
        ),
        ("nan raw", WIRE.replace(r#""raw":"41""#, r#""raw":"NaN""#)),
        (
            "long raw",
            WIRE.replace(r#""raw":"41""#, &format!(r#""raw":{raw33}"#)),
        ),
        ("effective digits", WIRE.replace("41.000000", "41.0")),
        (
            "derivation",
            WIRE.replace(r#""derivation":"raw""#, r#""derivation":"magic""#),
        ),
        (
            "uuid",
            WIRE.replace(
                "11111111-1111-4111-8111-111111111111",
                "11111111-1111-4111-8111-11111111111A",
            ),
        ),
        (
            "control",
            WIRE.replace(r#""producer":"s""#, r#""producer":"s\u0007""#),
        ),
        (
            "empty producer",
            WIRE.replace(r#""producer":"s""#, r#""producer":"""#),
        ),
        (
            "long bucket",
            WIRE.replace(r#""bucket":"weekly_all""#, &format!(r#""bucket":{long}"#)),
        ),
        (
            "policy version",
            WIRE.replace(r#""version":1"#, r#""version":2"#),
        ),
        (
            "credits word",
            WIRE.replace(r#""credits":"unreported""#, r#""credits":"lots""#),
        ),
        (
            "record size",
            WIRE.replace(
                r#""unknown_future_key":[1,2]"#,
                &format!(r#""unknown_future_key":"{pad}""#),
            ),
        ),
        ("not an object", "[1]".to_owned()),
    ];
    for (cause, line) in &refused {
        assert_ne!(line, WIRE, "fixture for {cause} must change the line");
        assert!(
            history::parse(line).is_err(),
            "{cause} must refuse: {line:.200}"
        );
    }
}

#[test]
fn a_hostile_workload_line_is_refused_for_each_single_cause() {
    let good = history::render(&Row::Workload(workload("s", INC[0], NOW, Some(NOW - 5))));
    assert!(
        history::parse(&good).is_ok(),
        "control workload parses: {good}"
    );
    for (cause, from, to) in [
        ("observed word", r#""busy""#, r#""sleepy""#),
        ("declared word", r#""working""#, r#""napping""#),
    ] {
        let line = good.replace(from, to);
        assert_ne!(line, good, "fixture for {cause}");
        assert!(
            history::parse(&line).is_err(),
            "{cause} must refuse: {line}"
        );
    }
    let mut crowd = workload("s", INC[0], NOW, Some(NOW - 5));
    crowd.seats = vec![crowd.seats[0].clone(); 129];
    let seats = history::render(&Row::Workload(crowd));
    assert!(history::parse(&seats).is_err(), "129 seats refuse");
}

#[test]
fn a_vendor_percent_past_one_hundred_is_kept_as_its_literal() {
    let line = WIRE
        .replace(r#""raw":"41""#, r#""raw":"100.4""#)
        .replace("41.000000", "100.400000");
    let Ok(Row::Sample(row)) = history::parse(&line) else {
        panic!("clamp nothing: {line}");
    };
    assert_eq!(
        (row.raw.as_str(), row.effective.as_str()),
        ("100.4", "100.400000")
    );
}

// ---------------------------------------------------------------- store

#[test]
fn append_writes_the_rows_to_a_private_dated_file_in_its_own_session() {
    let root = root("own");
    let dir = session(&root, "own", Some(INC[0]));
    let rows = [
        Row::Sample(sample("own", INC[0], NOW - 60, "41")),
        Row::Workload(workload("own", INC[0], NOW - 50, Some(NOW - 55))),
    ];
    history::append(&dir, &rows, NOW).expect("append");
    let file = dir
        .join("quota-history")
        .join(format!("{}.jsonl", date(NOW)));
    assert_eq!(mode(&dir.join("quota-history")), Some(0o700));
    assert_eq!(mode(&file), Some(0o600));
    assert_eq!(
        std::fs::read_to_string(&file).expect("day file"),
        lines_of(&rows)
    );
    let lock = dir.join("quota-history/.lock");
    if lock.exists() {
        assert_eq!(mode(&lock), Some(0o600), "the lock is private too");
    }
    let report = history::read_fleet(&root, NOW);
    assert_eq!(report.samples, vec![sample("own", INC[0], NOW - 60, "41")]);
    assert_eq!(
        report.workload,
        vec![workload("own", INC[0], NOW - 50, Some(NOW - 55))]
    );
    assert_eq!(report.skipped, Skipped::default());
}

#[test]
fn append_refuses_rows_that_are_not_its_own_sessions_and_writes_nothing() {
    let root = root("n7");
    let own = session(&root, "own", Some(INC[0]));
    let mixed = [
        Row::Sample(sample("own", INC[0], NOW - 60, "41")),
        Row::Sample(sample("own", INC[1], NOW - 60, "41")),
    ];
    assert!(
        history::append(&own, &mixed, NOW).is_err(),
        "a foreign row refuses the batch"
    );
    assert!(jsonl(&own).is_empty(), "nothing written");
    let anonymous = session(&root, "anon", None);
    let rows = [Row::Sample(sample("anon", INC[0], NOW - 60, "41"))];
    assert!(
        history::append(&anonymous, &rows, NOW).is_err(),
        "no session id, no history"
    );
    assert!(jsonl(&anonymous).is_empty());
}

#[test]
fn append_refuses_a_broad_dir_a_linked_dir_and_a_linked_day_file() {
    let root = root("safe");
    let rows = [Row::Sample(sample("a", INC[0], NOW - 60, "41"))];

    let broad = session(&root, "a", Some(INC[0]));
    std::fs::create_dir(broad.join("quota-history")).expect("dir");
    std::fs::set_permissions(
        broad.join("quota-history"),
        std::fs::Permissions::from_mode(0o755),
    )
    .expect("broad mode");
    assert!(
        history::append(&broad, &rows, NOW).is_err(),
        "a 0755 history dir refuses"
    );
    assert!(jsonl(&broad).is_empty());
    assert_eq!(
        mode(&broad.join("quota-history")),
        Some(0o755),
        "never chmods a node"
    );

    let elsewhere = root.join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("target");
    std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o700)).expect("mode");
    let linked = session(&root, "b", Some(INC[0]));
    std::os::unix::fs::symlink(&elsewhere, linked.join("quota-history")).expect("dir link");
    let rows_b = [Row::Sample(sample("b", INC[0], NOW - 60, "41"))];
    assert!(
        history::append(&linked, &rows_b, NOW).is_err(),
        "a linked dir refuses"
    );
    assert_eq!(std::fs::read_dir(&elsewhere).expect("target").count(), 0);

    // The link target is a valid, private, appendable day file: prove that
    // first through its own session, so only the link can refuse below.
    let real = session(&root, "real", Some(INC[0]));
    let held = [
        Row::Sample(sample("real", INC[0], NOW - 120, "40")),
        Row::Sample(sample("real", INC[0], NOW - 90, "40.5")),
    ];
    let target = plant(&real, NOW, &held[..1], "");
    history::append(&real, &held[1..], NOW).expect("the target itself is appendable");
    let kept = std::fs::read_to_string(&target).expect("target");
    assert_eq!(kept, lines_of(&held));
    assert_eq!(mode(&target), Some(0o600));
    let day = session(&root, "c", Some(INC[0]));
    let history_dir = day.join("quota-history");
    std::fs::create_dir(&history_dir).expect("dir");
    std::fs::set_permissions(&history_dir, std::fs::Permissions::from_mode(0o700)).expect("mode");
    let link = history_dir.join(format!("{}.jsonl", date(NOW)));
    std::os::unix::fs::symlink(&target, &link).expect("day link");
    let rows_c = [Row::Sample(sample("c", INC[0], NOW - 60, "41"))];
    assert!(
        history::append(&day, &rows_c, NOW).is_err(),
        "a linked day file refuses"
    );
    assert_eq!(std::fs::read_to_string(&target).expect("target"), kept);
    assert_eq!(mode(&target), Some(0o600));
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link")
            .file_type()
            .is_symlink()
    );
}

#[test]
fn append_seals_a_torn_tail_instead_of_joining_it_or_losing_the_day() {
    let root = root("seal");
    let dir = session(&root, "s", Some(INC[0]));
    let first = sample("s", INC[0], NOW - 120, "40");
    let fragment = r#"{"v":1,"kind":"sam"#;
    let file = plant(&dir, NOW, &[Row::Sample(first.clone())], fragment);
    let second = sample("s", INC[0], NOW - 60, "41");
    history::append(&dir, &[Row::Sample(second.clone())], NOW).expect("append after a torn tail");
    let text = std::fs::read_to_string(&file).expect("day file");
    assert_eq!(
        text.lines().nth(1),
        Some(fragment),
        "the fragment is sealed, never joined"
    );
    let report = history::read_fleet(&root, NOW);
    assert_eq!(report.samples, vec![first, second], "both rows survive");
    assert_eq!((report.skipped.damaged, report.skipped.partial), (1, 0));
}

#[test]
fn retention_keeps_today_and_seven_dates_deletes_older_and_ignores_other_names() {
    let root = root("keep");
    let dir = session(&root, "s", Some(INC[0]));
    let history_dir = dir.join("quota-history");
    std::fs::create_dir(&history_dir).expect("dir");
    std::fs::set_permissions(&history_dir, std::fs::Permissions::from_mode(0o700)).expect("mode");
    for back in -1..=9 {
        let path = history_dir.join(format!("{}.jsonl", date(NOW - back * DAY)));
        std::fs::write(&path, "\n").expect("day");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
    }
    std::fs::write(history_dir.join("notes.txt"), "mine\n").expect("unknown name");
    let rows = [Row::Sample(sample("s", INC[0], NOW - 60, "41"))];
    history::append(&dir, &rows, NOW).expect("an unknown name never refuses");
    for back in -1..=7 {
        assert!(
            history_dir
                .join(format!("{}.jsonl", date(NOW - back * DAY)))
                .exists(),
            "kept -{back}d"
        );
    }
    for back in 8..=9 {
        assert!(
            !history_dir
                .join(format!("{}.jsonl", date(NOW - back * DAY)))
                .exists(),
            "deleted -{back}d"
        );
    }
    assert_eq!(
        std::fs::read_to_string(history_dir.join("notes.txt")).expect("kept"),
        "mine\n"
    );
}

/// Three matched day files of valid private rows that differ only in size: one
/// byte under the cap after the batch (lands), one byte over (the batch's first
/// row alone would fit, so a partial write shows), and exactly at the cap.
#[test]
fn a_day_file_never_grows_past_its_cap_and_a_crossing_batch_adds_nothing() {
    let root = root("cap");
    let _gone = Gone(root.clone());
    let cap = n(CAP);
    for (name, inc) in [("lo", INC[0]), ("hi", INC[1]), ("at", INC[2])] {
        let dir = session(&root, name, Some(inc));
        let batch = [
            Row::Sample(sample(name, inc, NOW - 50, "42")),
            Row::Sample(sample(name, inc, NOW - 40, "43")),
        ];
        assert!(len_of(&batch[1]) > 2, "fixture rows render");
        let need = len_of(&batch[0]) + len_of(&batch[1]);
        let start = match name {
            "lo" => cap - need - 1,
            "hi" => cap - need + 1,
            _ => cap,
        };
        let file = plant(&dir, NOW, &[], &filler(name, inc, start));
        let before = std::fs::read(&file).expect("day file");
        assert_eq!(before.len(), start, "{name}: fixture size");
        let result = history::append(&dir, &batch, NOW);
        let after = std::fs::read(&file).expect("day file");
        if name == "lo" {
            assert!(result.is_ok(), "below the cap the batch lands: {result:?}");
            assert_eq!(after.len(), cap - 1);
            assert!(
                after[..start] == before[..] && after[start..] == *lines_of(&batch).as_bytes(),
                "lo: the batch is appended whole after the kept bytes"
            );
        } else {
            assert!(
                after == before,
                "{name}: no partial growth, no truncation ({} -> {} bytes)",
                before.len(),
                after.len()
            );
            assert_eq!(jsonl(&dir), vec![file.clone()], "{name}: no spill file");
        }
    }
}

#[test]
fn a_batch_over_its_row_or_byte_bound_is_refused_whole() {
    let root = root("batch");
    let samples = |name: &str, inc: &str, count: i64| -> Vec<Row> {
        (0..count)
            .map(|k| Row::Sample(sample(name, inc, NOW - 5000 + k, "41")))
            .collect()
    };
    let ok = session(&root, "r256", Some(INC[0]));
    let rows = samples("r256", INC[0], 256);
    assert!(history::append(&ok, &rows, NOW).is_ok(), "256 rows land");
    let file = ok
        .join("quota-history")
        .join(format!("{}.jsonl", date(NOW)));
    assert_eq!(std::fs::read_to_string(&file).ok(), Some(lines_of(&rows)));
    let over = session(&root, "r257", Some(INC[1]));
    assert!(
        history::append(&over, &samples("r257", INC[1], 257), NOW).is_err(),
        "257 rows refuse"
    );
    assert!(jsonl(&over).is_empty(), "nothing of a refused batch lands");

    let line = len_of(&Row::Workload(heavy("b1", INC[2], NOW - 100)));
    assert!(line > 40_000, "fixture rows render: {line}");
    let fit = (MIB - 1) / line;
    assert!(
        (fit + 1) * line > MIB && fit < 256,
        "the fixture straddles 1 MiB"
    );
    let heavies = |name: &str, inc: &str, count: usize| -> Vec<Row> {
        (0..count)
            .map(|k| {
                let back = i64::try_from(k).expect("small");
                Row::Workload(heavy(name, inc, NOW - 100 - back))
            })
            .collect()
    };
    let fits = session(&root, "b1", Some(INC[2]));
    let rows = heavies("b1", INC[2], fit);
    assert!(
        history::append(&fits, &rows, NOW).is_ok(),
        "under 1 MiB lands"
    );
    assert_eq!(jsonl(&fits).len(), 1);
    let big = session(&root, "b2", Some(INC[3]));
    assert!(
        history::append(&big, &heavies("b2", INC[3], fit + 1), NOW).is_err(),
        "over 1 MiB refuses"
    );
    assert!(jsonl(&big).is_empty(), "nothing of a refused batch lands");
}

/// 127 entries (room for the lock) append; 129 refuse before any write.
#[test]
fn append_inspects_a_bounded_history_dir_and_refuses_past_it() {
    let root = root("ents");
    for (name, inc, extra) in [("few", INC[0], 126), ("many", INC[1], 128)] {
        let dir = session(&root, name, Some(inc));
        let first = Row::Sample(sample(name, inc, NOW - 120, "40"));
        let second = Row::Sample(sample(name, inc, NOW - 60, "41"));
        let file = plant(&dir, NOW, std::slice::from_ref(&first), "");
        junk(&dir, extra);
        let result = history::append(&dir, std::slice::from_ref(&second), NOW);
        let after = std::fs::read_to_string(&file).expect("day file");
        if name == "few" {
            assert!(result.is_ok(), "127 entries append: {result:?}");
            assert_eq!(after, lines_of(&[first, second]));
        } else {
            assert!(result.is_err(), "129 entries refuse");
            assert_eq!(after, lines_of(&[first]), "nothing appended");
        }
    }
}

// ---------------------------------------------------------------- reader

#[test]
fn the_reader_salvages_lines_beside_damage_and_skips_a_torn_tail() {
    let root = root("salvage");
    let dir = session(&root, "s", Some(INC[0]));
    let a = sample("s", INC[0], NOW - 120, "40");
    let b = sample("s", INC[0], NOW - 60, "41");
    let body = format!(
        "{}\ngarbage{{\n{}\n{{\"v\":1",
        history::render(&Row::Sample(a.clone())),
        history::render(&Row::Sample(b.clone()))
    );
    plant(&dir, NOW, &[], &body);
    let report = history::read_fleet(&root, NOW);
    assert_eq!(report.samples, vec![a, b]);
    assert_eq!((report.skipped.damaged, report.skipped.partial), (1, 1));
}

#[test]
fn a_foreign_file_is_skipped_whole_and_the_fleet_read_goes_on() {
    let root = root("foreign");
    let mine = session(&root, "mine", Some(INC[0]));
    // Own rows before and after the foreign one: rejecting that row alone
    // would leak them, so only a whole-file skip passes.
    plant(
        &mine,
        NOW,
        &[
            Row::Sample(sample("mine", INC[0], NOW - 120, "40")),
            Row::Workload(workload("mine", INC[0], NOW - 100, Some(NOW - 105))),
            Row::Sample(sample("mine", INC[1], NOW - 60, "41")),
            Row::Sample(sample("mine", INC[0], NOW - 45, "44")),
            Row::Workload(workload("mine", INC[0], NOW - 40, Some(NOW - 42))),
        ],
        "",
    );
    let good = sample("mine", INC[0], NOW - DAY - 60, "39");
    plant(&mine, NOW - DAY, &[Row::Sample(good.clone())], "");
    let other = session(&root, "other", Some(INC[2]));
    let kept = sample("other", INC[2], NOW - 30, "42");
    plant(&other, NOW, &[Row::Sample(kept.clone())], "");
    let report = history::read_fleet(&root, NOW);
    assert_eq!(
        report.samples,
        vec![good, kept],
        "the session's other file and the other session stay"
    );
    assert!(report.workload.is_empty(), "{:?}", report.workload);
    assert_eq!(report.skipped.foreign, 1);
}

#[test]
fn links_dirs_oversize_and_unreadable_meta_are_skipped_per_file_and_counted() {
    let root = root("nodes");
    let good = sample("s", INC[0], NOW - 60, "41");
    let real = session(&root, "real", Some(INC[0]));
    let target = plant(&real, NOW, &[Row::Sample(good.clone())], "");
    let odd = session(&root, "odd", Some(INC[1]));
    let history_dir = odd.join("quota-history");
    std::fs::create_dir(&history_dir).expect("dir");
    private(&history_dir);
    std::os::unix::fs::symlink(&target, history_dir.join(format!("{}.jsonl", date(NOW))))
        .expect("link");
    std::fs::create_dir(history_dir.join(format!("{}.jsonl", date(NOW - DAY)))).expect("dir node");
    let big = std::fs::File::create(history_dir.join(format!("{}.jsonl", date(NOW - 2 * DAY))))
        .expect("big");
    big.set_len(CAP + 1).expect("sparse oversize");
    private(&history_dir.join(format!("{}.jsonl", date(NOW - 2 * DAY))));
    let blind = session(&root, "blind", Some(INC[2]));
    plant(
        &blind,
        NOW,
        &[Row::Sample(sample("blind", INC[2], NOW - 60, "43"))],
        "",
    );
    std::fs::set_permissions(blind.join("meta"), std::fs::Permissions::from_mode(0o000))
        .expect("an unreadable meta");
    let report = history::read_fleet(&root, NOW);
    assert_eq!(
        report.samples,
        vec![good],
        "only the real session's file is read"
    );
    let s = &report.skipped;
    assert_eq!(
        (s.link, s.nonregular, s.oversize, s.unreadable),
        (1, 1, 1, 1),
        "{s:?}"
    );
}

#[test]
fn a_renamed_session_keeps_its_history_by_incarnation() {
    let root = root("rename");
    let old = session(&root, "old", Some(INC[0]));
    let row = sample("old", INC[0], NOW - 60, "41");
    history::append(&old, &[Row::Sample(row.clone())], NOW).expect("append");
    let new = root.join("sessions").join("new");
    std::fs::rename(&old, &new).expect("rename moves the dir and keeps session_id");
    std::fs::write(
        new.join("meta"),
        format!("session=new\nsession_id={}\n", INC[0]),
    )
    .expect("meta");
    let report = history::read_fleet(&root, NOW);
    assert_eq!(
        report.samples,
        vec![row],
        "producer stays the name at write time"
    );
    assert_eq!(report.skipped.foreign, 0);
}

#[test]
fn the_reader_changes_nothing_on_disk() {
    let root = root("ro");
    let dir = session(&root, "s", Some(INC[0]));
    plant(
        &dir,
        NOW,
        &[Row::Sample(sample("s", INC[0], NOW - 60, "41"))],
        "{\"torn",
    );
    plant(&dir, NOW - 9 * DAY, &[], "\n");
    let snapshot = |path: &Path| -> Vec<(PathBuf, u64, u32, std::time::SystemTime)> {
        let mut out: Vec<_> = std::fs::read_dir(path.join("quota-history"))
            .expect("dir")
            .filter_map(Result::ok)
            .map(|entry| {
                let meta = entry.metadata().expect("node");
                (
                    entry.path(),
                    meta.len(),
                    meta.permissions().mode(),
                    meta.modified().expect("mtime"),
                )
            })
            .collect();
        out.sort();
        out
    };
    let before = snapshot(&dir);
    let report = history::read_fleet(&root, NOW);
    assert_eq!(report.samples.len(), 1, "the read happened");
    assert_eq!(
        snapshot(&dir),
        before,
        "no delete, seal, lock or mode change"
    );
}

#[test]
fn rows_with_impossible_clocks_are_refused_as_damage() {
    let root = root("clock");
    let dir = session(&root, "s", Some(INC[0]));
    let good = sample("s", INC[0], NOW - 60, "41");
    let mut ahead = sample("s", INC[0], NOW - 30, "42");
    ahead.observed_at = ahead.collected_at + 300;
    let mut future = sample("s", INC[0], NOW - 20, "43");
    future.collected_at = NOW + 300;
    let backwards = workload("s", INC[0], NOW - 10, Some(NOW - 5));
    plant(
        &dir,
        NOW,
        &[
            Row::Sample(good.clone()),
            Row::Sample(ahead),
            Row::Sample(future),
            Row::Workload(backwards),
        ],
        "",
    );
    let report = history::read_fleet(&root, NOW);
    assert_eq!(report.samples, vec![good]);
    assert!(report.workload.is_empty());
    assert_eq!(report.skipped.damaged, 3);
}

/// 128 history entries are read whole; a 129th is a counted truncation.
#[test]
fn the_fleet_read_counts_a_history_dir_past_its_entry_bound() {
    for (tag, extra) in [("e128", 127), ("e129", 128)] {
        let root = root(tag);
        let dir = session(&root, "s", Some(INC[0]));
        let row = sample("s", INC[0], NOW - 60, "41");
        plant(&dir, NOW, &[Row::Sample(row.clone())], "");
        junk(&dir, extra);
        let report = history::read_fleet(&root, NOW);
        if extra == 127 {
            assert_eq!(report.samples, vec![row]);
            assert_eq!(report.skipped, Skipped::default());
        } else {
            assert!(report.skipped.truncated >= 1, "{:?}", report.skipped);
        }
    }
}

/// 128 session dirs are read; the 129th is a counted truncation.
#[test]
fn the_fleet_read_counts_sessions_past_its_session_bound() {
    let root = root("wide");
    let add = |k: usize| {
        let name = format!("f{k:03}");
        let inc = uuid(k);
        let dir = session(&root, &name, Some(&inc));
        let back = i64::try_from(k).expect("small");
        let row = sample(&name, &inc, NOW - 1000 + back, "41");
        plant(&dir, NOW, &[Row::Sample(row)], "");
    };
    (0..128).for_each(&add);
    let report = history::read_fleet(&root, NOW);
    assert_eq!(
        (report.samples.len(), report.skipped.truncated),
        (128, 0),
        "{:?}",
        report.skipped
    );
    add(128);
    let report = history::read_fleet(&root, NOW);
    assert!(
        report.skipped.truncated >= 1 && report.samples.len() <= 128,
        "{} samples, {:?}",
        report.samples.len(),
        report.skipped
    );
}

/// 65536 distinct samples are returned; one more is a counted truncation.
#[test]
fn the_fleet_read_counts_rows_past_its_output_bound() {
    let root = root("rows");
    let _gone = Gone(root.clone());
    for (k, home) in ["/h1", "/h2", "/h3"].iter().enumerate() {
        let name = format!("o{k}");
        let count = if k == 2 { 1 } else { 32_768 };
        let rows: Vec<Row> = (0..count)
            .map(|step| {
                let mut row = sample(&name, INC[k], NOW - 40_000 + step, "41");
                row.scope = scope(home);
                Row::Sample(row)
            })
            .collect();
        let dir = session(&root, &name, Some(INC[k]));
        if k < 2 {
            plant(&dir, NOW, &rows, "");
            continue;
        }
        let report = history::read_fleet(&root, NOW);
        assert_eq!(
            (report.samples.len(), report.skipped.truncated),
            (65_536, 0),
            "{:?}",
            report.skipped
        );
        plant(&dir, NOW, &rows, "");
    }
    let report = history::read_fleet(&root, NOW);
    assert!(
        report.skipped.truncated >= 1 && report.samples.len() <= 65_536,
        "{} samples, {:?}",
        report.samples.len(),
        report.skipped
    );
}

/// Four 14 MiB sessions (56 MiB) are read whole; a fifth (70 MiB) passes the
/// 64 MiB fleet bound and is a counted truncation.
#[test]
fn the_fleet_read_counts_bytes_past_its_fleet_bound() {
    let root = root("bytes");
    let _gone = Gone(root.clone());
    let line = len_of(&Row::Workload(heavy("w0", INC[0], NOW - 100)));
    assert!(line > 40_000, "fixture rows render: {line}");
    let per = 14 * MIB / line;
    let mut planted = 0;
    for (k, inc) in INC.iter().take(5).enumerate() {
        if k == 4 {
            let report = history::read_fleet(&root, NOW);
            assert_eq!(
                (report.workload.len(), report.skipped.truncated),
                (planted, 0),
                "{:?}",
                report.skipped
            );
        }
        let name = format!("w{k}");
        let rows: Vec<Row> = (0..per)
            .map(|step| {
                let back = i64::try_from(step).expect("small");
                Row::Workload(heavy(&name, inc, NOW - 100 - back))
            })
            .collect();
        let dir = session(&root, &name, Some(inc));
        plant(&dir, NOW, &rows, "");
        planted += per;
    }
    let report = history::read_fleet(&root, NOW);
    assert!(
        report.skipped.truncated >= 1 && report.workload.len() < planted,
        "{} of {planted} rows, {:?}",
        report.workload.len(),
        report.skipped
    );
}

// ---------------------------------------------------------------- fold

#[test]
fn one_vendor_sample_seen_by_many_sessions_is_one_sample_never_a_sum() {
    let root = root("dedupe");
    let one = session(&root, "a", Some(INC[0]));
    let two = session(&root, "b", Some(INC[1]));
    let mut late = sample("b", INC[1], NOW - 600, "41");
    late.collected_at = NOW - 100;
    plant(
        &one,
        NOW,
        &[
            Row::Sample(sample("a", INC[0], NOW - 600, "41")),
            Row::Sample(sample("a", INC[0], NOW - 60, "44")),
        ],
        "",
    );
    plant(
        &two,
        NOW,
        &[
            Row::Sample(late),
            Row::Sample(sample("b", INC[1], NOW - 300, "43")),
        ],
        "",
    );
    let report = history::read_fleet(&root, NOW);
    let mut seen: Vec<(i64, &str)> = report
        .samples
        .iter()
        .map(|s| (s.observed_at, s.raw.as_str()))
        .collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![(NOW - 600, "41"), (NOW - 300, "43"), (NOW - 60, "44")]
    );
    let tie = root.join("tie");
    let three = session(&tie, "c", Some(INC[2]));
    let four = session(&tie, "d", Some(INC[3]));
    plant(
        &three,
        NOW,
        &[Row::Sample(sample("c", INC[2], NOW - 60, "41"))],
        "",
    );
    plant(
        &four,
        NOW,
        &[Row::Sample(sample("d", INC[3], NOW - 60, "43"))],
        "",
    );
    let report = history::read_fleet(&tie, NOW);
    assert_eq!(report.samples.len(), 1, "{report:?}");
    assert_eq!(
        report.samples[0].raw, "43",
        "a conflicting tie keeps the highest raw"
    );
}

/// One vendor clock, many identities: only an exact duplicate collapses.
#[test]
fn same_clock_samples_that_differ_in_any_identity_stay_distinct() {
    let root = root("ident");
    let base = |name: &str, inc: &str| sample(name, inc, NOW - 60, "41");
    let one = session(&root, "a", Some(INC[0]));
    plant(&one, NOW, &[Row::Sample(base("a", INC[0]))], "");
    let vary = |change: fn(&mut Sample)| {
        let mut row = base("b", INC[1]);
        change(&mut row);
        row
    };
    let variants = [
        vary(|s| s.bucket = "session".to_owned()),
        vary(|s| s.qualifier = Some("opus".to_owned())),
        vary(|s| s.window_minutes = Some(300)),
        vary(|s| s.window_minutes = None),
        vary(|s| s.scope.tool = "codex".to_owned()),
        vary(|s| s.scope.source = "/h/other.json".to_owned()),
        vary(|s| s.scope.account = None),
        vary(|s| s.resets_at = RESET + DAY),
        vary(|s| s.policy.manual_resets = Some(1)),
        vary(|s| s.policy.credits = "unlimited".to_owned()),
        vary(|s| s.policy.spend_cap = Some(true)),
    ];
    let mut rows = vec![Row::Sample(base("b", INC[1]))];
    rows.extend(variants.iter().cloned().map(Row::Sample));
    let two = session(&root, "b", Some(INC[1]));
    plant(&two, NOW, &rows, "");
    let report = history::read_fleet(&root, NOW);
    assert_eq!(
        report.samples.len(),
        1 + variants.len(),
        "{:?}",
        report.samples
    );
    assert!(
        report.samples.contains(&base("a", INC[0])) && !report.samples.contains(&base("b", INC[1])),
        "the exact duplicate collapses onto the lexically first producer"
    );
    for variant in &variants {
        assert!(report.samples.contains(variant), "kept apart: {variant:?}");
    }
}

#[test]
fn local_policies_are_separate_tracks_and_never_cut_each_other() {
    let root = root("policy");
    let a = session(&root, "a", Some(INC[0]));
    let b = session(&root, "b", Some(INC[1]));
    let track = |name: &str, inc: &str, resets: Option<u8>, clocks: [i64; 2]| -> Vec<Row> {
        clocks
            .iter()
            .map(|clock| {
                let mut row = sample(name, inc, *clock, "41");
                row.policy = policy(resets);
                Row::Sample(row)
            })
            .collect()
    };
    plant(
        &a,
        NOW,
        &track("a", INC[0], None, [NOW - 400, NOW - 200]),
        "",
    );
    plant(
        &b,
        NOW,
        &track("b", INC[1], Some(1), [NOW - 400, NOW - 100]),
        "",
    );
    let report = history::read_fleet(&root, NOW);
    assert_eq!(
        report.samples.len(),
        4,
        "one clock under two policies is two pieces of evidence"
    );
    assert!(
        segments(&report).is_empty(),
        "interleaved producers cut nothing: {:?}",
        report.segments
    );
}

#[test]
fn inside_one_track_window_beats_policy_beats_drop_and_the_band_is_inclusive() {
    let root = root("segs");
    let dir = session(&root, "s", Some(INC[0]));
    let t = |n: i64| NOW - 3000 + n * 100;
    let mut rows: Vec<Row> = [(1, "40"), (2, "39"), (3, "37"), (4, "30"), (5, "31")]
        .iter()
        .map(|(n, raw)| Row::Sample(sample("s", INC[0], t(*n), raw)))
        .collect();
    let mut six = sample("s", INC[0], t(6), "32");
    six.policy = policy(Some(1));
    let mut seven = six.clone();
    seven.observed_at = t(7);
    seven.collected_at = t(7) + 10;
    seven.resets_at = RESET + 7 * DAY;
    seven.raw = "1".to_owned();
    seven.effective = "1.000000".to_owned();
    let mut eight = seven.clone();
    eight.observed_at = t(8);
    eight.collected_at = t(8) + 10;
    eight.raw = "0.5".to_owned();
    eight.effective = "0.500000".to_owned();
    rows.extend([six, seven, eight].map(Row::Sample));
    plant(&dir, NOW, &rows, "");
    let report = history::read_fleet(&root, NOW);
    assert_eq!(report.samples.len(), 8);
    assert_eq!(
        segments(&report),
        vec![
            (t(2), "drop".to_owned(), true),
            (t(3), "drop".to_owned(), true),
            (t(4), "drop".to_owned(), false),
            (t(6), "policy".to_owned(), false),
            (t(7), "window".to_owned(), false),
            (t(8), "drop".to_owned(), true),
        ]
    );
    let _ = at(&report, t(1));
}

#[test]
fn two_claude_homes_in_one_session_are_two_silent_tracks() {
    let root = root("homes");
    let dir = session(&root, "s", Some(INC[0]));
    let rows: Vec<Row> = [
        ("/h1", 1, "50"),
        ("/h2", 2, "10"),
        ("/h1", 3, "51"),
        ("/h2", 4, "11"),
    ]
    .iter()
    .map(|(home, n, raw)| {
        let mut row = sample("s", INC[0], NOW - 1000 + n * 100, raw);
        row.scope = scope(home);
        Row::Sample(row)
    })
    .collect();
    plant(&dir, NOW, &rows, "");
    let report = history::read_fleet(&root, NOW);
    assert_eq!(report.samples.len(), 4);
    assert!(
        segments(&report).is_empty(),
        "no drop, window or policy between accounts: {:?}",
        report.segments
    );
}

#[test]
fn two_producers_reporting_one_discontinuity_report_it_once() {
    let root = root("segdedupe");
    for (name, inc) in [("a", INC[0]), ("b", INC[1])] {
        let dir = session(&root, name, Some(inc));
        let rows = [(NOW - 300, "40"), (NOW - 200, "30")]
            .map(|(clock, raw)| Row::Sample(sample(name, inc, clock, raw)));
        plant(&dir, NOW, &rows, "");
    }
    let report = history::read_fleet(&root, NOW);
    assert_eq!(report.samples.len(), 2);
    assert_eq!(
        segments(&report),
        vec![(NOW - 200, "drop".to_owned(), false)]
    );
}

#[test]
fn workload_aligns_on_world_clocks_and_a_missing_session_stays_absent() {
    assert_eq!(ALIGN_SECS, 900);
    let root = root("align");
    let t = NOW - 2000;
    let vendor = session(&root, "v", Some(INC[6]));
    let mut lone = sample("v", INC[6], t + 1500, "45");
    lone.collected_at = t + 1510;
    plant(
        &vendor,
        NOW,
        &[Row::Sample(sample("v", INC[6], t, "41")), Row::Sample(lone)],
        "",
    );
    let mut other_scope = workload("f", INC[5], t - 5, Some(t - 10));
    other_scope.scope = scope("/elsewhere");
    let rows = [
        (
            "a",
            INC[0],
            vec![
                workload("a", INC[0], t - 290, Some(t - 300)),
                workload("a", INC[0], t - 90, Some(t - 100)),
            ],
        ),
        (
            "b",
            INC[1],
            vec![workload("b", INC[1], t - 990, Some(t - 1000))],
        ),
        (
            "c",
            INC[2],
            vec![workload("c", INC[2], t + 20, Some(t + 10))],
        ),
        ("d", INC[3], vec![workload("d", INC[3], t, None)]),
        (
            "e",
            INC[4],
            vec![workload("e", INC[4], t + 900, Some(t - 50))],
        ),
        ("f", INC[5], vec![other_scope]),
    ];
    for (name, inc, list) in rows {
        let dir = session(&root, name, Some(inc));
        let list: Vec<Row> = list.into_iter().map(Row::Workload).collect();
        plant(&dir, NOW, &list, "");
    }
    let report = history::read_fleet(&root, NOW);
    assert_eq!(report.samples.len(), 2);
    assert_eq!(
        report.aligned.len(),
        report.samples.len(),
        "one association per sample"
    );
    let first = &report.aligned[at(&report, t)];
    let chosen: Vec<(&str, Option<i64>)> = first
        .iter()
        .map(|index| {
            (
                report.workload[*index].incarnation.as_str(),
                report.workload[*index].observed_at,
            )
        })
        .collect();
    assert_eq!(
        chosen,
        vec![(INC[0], Some(t - 100))],
        "newest in-bound row of the one live session"
    );
    assert!(
        report.aligned[at(&report, t + 1500)].is_empty(),
        "no row in bound is ABSENT, not zero busy"
    );
    assert!(
        report
            .workload
            .iter()
            .all(|row| !row.seats.is_empty() || row.unscoped > 0 || row.observed_at.is_none()),
        "no synthesized empty row stands in for a missing session: {:?}",
        report.workload
    );
}

// ---------------------------------------------------------------- daemon

struct Daemon {
    child: super::cli::OwnedChild,
    trace: PathBuf,
    log: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct World {
    root: PathBuf,
    home: PathBuf,
    sock: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = root(tag);
        let home = root.join("home");
        std::fs::create_dir_all(&home).expect("tool home");
        let home = std::fs::canonicalize(home).expect("canonical home");
        std::fs::write(root.join("config"), "[profiles]\ncl = claude\n").expect("config");
        assert!(
            super::phase2::tmux_present(&root),
            "tmux is required for the watchdog arms"
        );
        Self {
            sock: root.join("s"),
            root,
            home,
        }
    }
    fn tmux(&self, words: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.sock.clone()),
        ));
        args.extend(words.iter().map(|word| (*word).to_owned()));
        super::phase2::run_tmux(&args, &self.root)
    }
    /// A watched session running `cat`, its lead pane stamped; meta extra rows appended.
    fn watched(&self, name: &str, extra: &str) -> (PathBuf, String) {
        let dir = self.root.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("session dir");
        std::fs::write(
            dir.join("meta"),
            format!(
                "mode=local\nsession={name}\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nprofile.main=cl\nagent_bin.main=claude\nconfig_home.main={}\n{extra}",
                self.sock.display(),
                self.home.display()
            ),
        )
        .expect("meta");
        assert!(
            self.tmux(&["new-session", "-d", "-s", name, "cat"]).0,
            "session {name}"
        );
        let (_, pane) = self.tmux(&["display-message", "-p", "-t", name, "#{pane_id}"]);
        let pane = pane.trim().to_owned();
        for (option, value) in [("@ae_agent", "lead"), ("@ae_slot", "main")] {
            assert!(
                self.tmux(&["set-option", "-p", "-t", &pane, option, value])
                    .0
            );
        }
        (dir, pane)
    }
    fn quota(&self, session: u8, week: u8, observed_at: i64) {
        let reset = ae::time::Timestamp::from_epoch(observed_at + 7_200);
        std::fs::write(
            self.home.join(".claude.json"),
            format!(
                "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"group\":\"session\",\"percent\":{session},\"resets_at\":\"{reset}\",\"scope\":null}},{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":{week},\"resets_at\":\"{reset}\",\"scope\":null}}]}}}}}}\n",
                observed_at * 1_000
            ),
        )
        .expect("claude cache");
    }
    fn daemon(&self, dir: &Path, interval: &str, every: &str) -> Daemon {
        let trace = dir.with_extension("trace");
        std::fs::write(&trace, "").expect("trace");
        let log = dir.with_extension("err");
        let err = std::fs::File::create(&log).expect("daemon log");
        let child = super::cli::ae()
            .arg("_watchdog-run")
            .arg(dir)
            .args(["--interval", interval])
            .args(["--stale-secs", "999999", "--tg-supervise-secs", "0"])
            .args(["--quota-every-secs", every])
            .env("HOME", &self.home)
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TEST_QUOTA_TRACE", &trace)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stdout(std::process::Stdio::null())
            .stderr(err)
            .spawn()
            .expect("real watchdog");
        Daemon { child, trace, log }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
    }
}

fn traced(daemon: &Daemon, word: &str, count: usize) {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        let text = std::fs::read_to_string(&daemon.trace).unwrap_or_default();
        if text.lines().filter(|line| line.starts_with(word)).count() >= count {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "no {count} `{word}` trace lines: {}",
        daemon.trace.display()
    );
}

/// Quota passes the daemon has finished: its `observed` trace lines.
fn passes(daemon: &Daemon) -> usize {
    std::fs::read_to_string(&daemon.trace)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("observed "))
        .count()
}

/// The rows the history holds right now: complete lines that parse.
fn rows_now(dir: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    for file in jsonl(dir) {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let complete = text.rfind('\n').map_or("", |end| &text[..end]);
        rows.extend(
            complete
                .lines()
                .filter_map(|line| history::parse(line).ok()),
        );
    }
    rows
}

/// The workload rows among `rows`, oldest collection first.
fn workloads(rows: &[Row]) -> Vec<&Workload> {
    let mut work: Vec<&Workload> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Workload(w) => Some(w),
            Row::Sample(_) => None,
        })
        .collect();
    work.sort_by_key(|w| w.collected_at);
    work
}

/// Bounded readiness on the history's own EVIDENCE, never on a pass count:
/// wait until `done` holds, and fail loudly — rows, trace and stderr — once
/// the daemon exits, `budget` further quota passes go by, or `WAIT` runs out.
fn until(
    daemon: &mut Daemon,
    dir: &Path,
    budget: usize,
    done: impl Fn(&[Row]) -> bool,
) -> Vec<Row> {
    let deadline = Instant::now() + WAIT;
    let base = passes(daemon);
    loop {
        let rows = rows_now(dir);
        if done(&rows) {
            return rows;
        }
        let exited = daemon.child.try_wait().ok().flatten();
        let ran = passes(daemon).saturating_sub(base);
        assert!(
            exited.is_none() && ran < budget && Instant::now() < deadline,
            "history evidence missing after {ran} quota passes (exit {exited:?}): {rows:?}\ntrace:\n{}\nstderr:\n{}",
            std::fs::read_to_string(&daemon.trace).unwrap_or_default(),
            std::fs::read_to_string(&daemon.log).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Every complete line, strictly: a killed daemon may leave only its tail torn.
fn file_rows(dir: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    for file in jsonl(dir) {
        let text = std::fs::read_to_string(file).expect("day");
        let complete = text.rfind('\n').map_or("", |end| &text[..end]);
        for line in complete.lines() {
            rows.push(history::parse(line).unwrap_or_else(|why| panic!("{why:?}: {line}")));
        }
    }
    rows
}

/// The daemon's vendor rows: one per changed sample, its identities and clocks.
fn vendor_rows(rows: &[Row], source: &str, start: i64, end: i64) {
    let samples: Vec<&Sample> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Sample(s) => Some(s),
            Row::Workload(_) => None,
        })
        .collect();
    let session_rows: Vec<&&Sample> = samples.iter().filter(|s| s.bucket == "session").collect();
    assert_eq!(
        session_rows.len(),
        1,
        "an unchanged vendor sample is written once: {samples:?}"
    );
    let s = session_rows[0];
    assert_eq!(
        (s.producer.as_str(), s.incarnation.as_str()),
        ("qhd", INC[0])
    );
    assert_eq!(
        (s.scope.tool.as_str(), s.scope.source.as_str()),
        ("claude", source)
    );
    assert_eq!(
        (s.raw.as_str(), s.effective.as_str(), s.derivation.as_str()),
        ("41", "41.000000", "raw")
    );
    assert_eq!(
        (s.window_minutes, s.observed_at, s.resets_at),
        (Some(300), start, start + 7_200)
    );
    assert!(
        (start..=end).contains(&s.collected_at),
        "collection is this pass's clock"
    );
    assert!(
        samples
            .iter()
            .any(|s| s.bucket == "weekly_all" && s.raw == "17")
    );
}

/// Consecutive workload rows whose state clock is at or after the previous
/// pass's collection: each one is a fresh capture of the prior cycle.
fn advancing(work: &[&Workload]) -> usize {
    work.windows(2)
        .filter(|pair| {
            pair[1]
                .observed_at
                .is_some_and(|at| at >= pair[0].collected_at)
        })
        .count()
}

/// Ready once two captures advanced: a clock captured once and never
/// refreshed meets the bound only on the pair right after that capture.
fn two_fresh_captures(rows: &[Row]) -> bool {
    advancing(&workloads(rows)) >= 2
}

/// The daemon's workload rows: one per pass, prior-cycle state, declared state.
/// `all` is the whole history after the daemon stopped: with a 2 s interval
/// the capture of cycle k lands at or after pass k's collection and at least
/// 2 s before pass k+1's, so no state clock, later rows included, may equal
/// or pass its own row's collection. A later row with no clock (a failed
/// capture) is allowed. `ready` is the snapshot that proved two fresh
/// captures; the seat is judged on its newest clocked row.
fn workload_rows(ready: &[Row], all: &[Row], source: &str) {
    let work = workloads(all);
    assert!(work.len() >= 3, "one workload row per pass: {work:?}");
    for row in &work {
        if let Some(state_at) = row.observed_at {
            assert!(
                state_at < row.collected_at,
                "a state clock is a prior capture's, never this pass's: {row:?}"
            );
            assert!(
                state_at >= work[0].collected_at,
                "no clock from before this daemon's first pass: {row:?}"
            );
        }
    }
    assert!(
        work.iter()
            .all(|w| w.producer == "qhd" && w.incarnation == INC[0] && w.scope.source == source)
    );
    assert_eq!(
        work[0].observed_at, None,
        "the first pass has no prior cycle"
    );
    assert!(
        work[0].seats.iter().all(|seat| seat.observed == "unknown"),
        "never idle by default"
    );
    let proved = workloads(ready);
    assert!(advancing(&proved) >= 2, "the ready snapshot: {proved:?}");
    let last = proved
        .iter()
        .rev()
        .find(|w| w.observed_at.is_some())
        .expect("the ready snapshot holds clocked rows");
    let state_at = last.observed_at.unwrap_or_default();
    assert!(last.collected_at - state_at <= ALIGN_SECS);
    let seat = last
        .seats
        .iter()
        .find(|seat| seat.slot == "main")
        .expect("the proven lead seat");
    assert_eq!(
        (seat.agent.as_str(), seat.declared.as_deref()),
        ("lead", Some("waiting-agent"))
    );
    assert!(["busy", "idle", "unknown"].contains(&seat.observed.as_str()));
    assert_eq!(last.unscoped, 0);
}

#[test]
fn the_watchdog_appends_its_own_sessions_samples_once_and_workload_every_pass() {
    let world = World::new("daemon");
    let (dir, pane) = world.watched("qhd", &format!("session_id={}\n", INC[0]));
    let peer = session(&world.root, "qhpeer", Some(INC[1]));
    let start = ae::time::Timestamp::now().epoch();
    world.quota(41, 17, start);
    let declared = super::cli::ae()
        .args(["@qhd", "state", "waiting-agent", "quotahist-nav fixture waits on nobody real: this reason only satisfies the 80-600 character rule for the spec"])
        .env("TMUX", format!("{},0,0", world.sock.display()))
        .env("TMUX_PANE", &pane)
        .env("AE_HOME", &world.root)
        .env("HOME", &world.home)
        .env("CONFIG_FILE", world.root.join("config"))
        .output()
        .expect("state helper");
    assert_eq!(declared.status.code(), Some(0), "{declared:?}");
    let mut daemon = world.daemon(&dir, "2", "1");
    let ready = until(&mut daemon, &dir, 12, two_fresh_captures);
    drop(daemon);
    let end = ae::time::Timestamp::now().epoch();

    assert_eq!(
        mode(&dir.join("quota-history")),
        Some(0o700),
        "the history dir exists and is private"
    );
    for file in jsonl(&dir) {
        assert_eq!(mode(&file), Some(0o600));
        let name = file
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        assert!(
            name == format!("{}.jsonl", date(start)) || name == format!("{}.jsonl", date(end)),
            "{name}"
        );
    }
    let rows = file_rows(&dir);
    let source = world.home.join(".claude.json").display().to_string();
    vendor_rows(&rows, &source, start, end);
    workload_rows(&ready, &rows, &source);
    assert!(!peer.join("quota-history").exists(), "own session only");
    let report = history::read_fleet(&world.root, end);
    let mut buckets: Vec<(&str, &str, i64)> = report
        .samples
        .iter()
        .map(|s| (s.bucket.as_str(), s.raw.as_str(), s.observed_at))
        .collect();
    buckets.sort_unstable();
    assert_eq!(
        buckets,
        vec![("session", "41", start), ("weekly_all", "17", start)],
        "one vendor clock, two buckets, two samples through the fleet read"
    );
}

/// A seat whose pane goes away is no longer captured: the rows after it keep
/// the older state clock (or none), never one restamped by a later pass.
#[test]
fn a_capture_that_stops_never_restamps_the_state_clock() {
    let world = World::new("gone");
    let (dir, pane) = world.watched("qhgone", &format!("session_id={}\n", INC[0]));
    assert!(
        world.tmux(&["new-window", "-d", "-t", "qhgone", "cat"]).0,
        "a second window keeps the session alive"
    );
    world.quota(41, 17, ae::time::Timestamp::now().epoch());
    let mut daemon = world.daemon(&dir, "1", "1");
    until(&mut daemon, &dir, 12, |rows| {
        workloads(rows).iter().any(|w| w.observed_at.is_some())
    });
    assert!(
        world.tmux(&["kill-pane", "-t", &pane]).0,
        "the seat's pane goes"
    );
    let gone = ae::time::Timestamp::now().epoch();
    let rows = until(&mut daemon, &dir, 12, |rows| {
        workloads(rows)
            .iter()
            .filter(|w| w.collected_at > gone + 1)
            .count()
            >= 2
    });
    drop(daemon);
    for row in workloads(&rows)
        .into_iter()
        .filter(|w| w.collected_at > gone + 1)
    {
        assert!(
            row.observed_at.is_none_or(|at| at <= gone),
            "a skipped capture keeps the older clock or none: {row:?}"
        );
        assert!(
            row.seats.iter().all(|seat| seat.observed == "unknown"),
            "no capture, no busy or idle: {row:?}"
        );
    }
}

/// CONTROL: no pass, no history — off, zero cadence, no session id, and a
/// history path that cannot be written never stop the pass.
#[test]
fn no_quota_pass_no_history_and_a_failed_write_never_breaks_the_pass() {
    let world = World::new("quiet");
    world.quota(41, 17, ae::time::Timestamp::now().epoch());
    let id = |n: usize| format!("session_id={}\n", INC[n]);
    let (off, _) = world.watched(
        "qhoff",
        &format!("{}quota=off\nquota_every_secs=1\n", id(0)),
    );
    let (zero, _) = world.watched("qhzero", &id(1));
    let (anon, _) = world.watched("qhanon", "");
    let (jam, _) = world.watched("qhjam", &id(3));
    std::fs::write(jam.join("quota-history"), "not a dir\n").expect("blocking node");
    let daemons = [
        world.daemon(&off, "1", "1"),
        world.daemon(&zero, "1", "0"),
        world.daemon(&anon, "1", "1"),
        world.daemon(&jam, "1", "1"),
    ];
    traced(&daemons[0], "skipped", 3);
    traced(&daemons[2], "observed ", 3);
    traced(&daemons[3], "observed ", 3);
    for daemon in &daemons {
        let text = std::fs::read_to_string(&daemon.trace).unwrap_or_default();
        for line in text.lines().filter(|line| line.starts_with("observed ")) {
            let mut words = line.split(' ');
            assert_eq!(words.next(), Some("observed"));
            assert!(
                words.next().is_some_and(|w| w.starts_with("max=")),
                "{line}"
            );
            assert!(
                words.next().is_some_and(|w| w
                    .strip_prefix("booked=")
                    .is_some_and(|n| n.parse::<usize>().is_ok())),
                "{line}"
            );
            assert_eq!(words.next(), None, "trace bytes unchanged: {line}");
        }
    }
    assert!(
        std::fs::read_to_string(&daemons[1].trace)
            .unwrap_or_default()
            .is_empty(),
        "cadence 0 runs no pass"
    );
    for dir in [&off, &zero, &anon] {
        assert!(!dir.join("quota-history").exists(), "{}", dir.display());
    }
    assert_eq!(
        std::fs::read_to_string(jam.join("quota-history")).expect("node"),
        "not a dir\n"
    );
}

/// CONTROL: `ae quota` reads no history and prints the same bytes beside it.
#[test]
fn ae_quota_output_is_byte_identical_beside_a_history() {
    let world = World::new("table");
    let now = ae::time::Timestamp::now().epoch();
    world.quota(41, 17, now);
    let table = || {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = ae::quota::run(
            &ae::quota::Inputs {
                home: Some(&world.home),
                global: Some(&world.root.join("config")),
                local: None,
                sessions: Some(&world.root.join("sessions")),
                now,
            },
            &mut out,
            &mut err,
        )
        .expect("quota read");
        (code, out, err)
    };
    std::fs::create_dir_all(world.root.join("sessions")).expect("sessions");
    let before = table();
    let dir = session(&world.root, "s", Some(INC[0]));
    plant(&dir, now, &[], "{\"v\":1,\"kind\":\"sample\"}\n");
    assert_eq!(table(), before);
}
