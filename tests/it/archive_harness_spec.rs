//! Frozen acceptance (#248 P1a): the archive's purge record `harness.tsv`.
//!
//! Oracle: #248 "keep `profile`/`agent_bin`/`config_home` (+ base)/`harness_session[_prior]`
//! per slot as `<tool>:<uuid>` in a separate inert 0600 file, read only by purge,
//! never by `--from`", lead rulings B2 (six home states, three base states, never
//! collapsed) and I1 (escaped atoms, comma priors, tool grammar), and plan r2 L5-L12.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures write sessions and read published archives on disk"
)]

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ae::archive::harness::{self, Conversation};
use ae::meta::{RecordedConfigHome, RecordedConfigHomeBase};

const AID: &str = "a1b2c3d4-0000-4000-8000-000000000001";
const AT: &str = "2026-08-28T10:00:00Z";
const CUR: &str = "0199c0de-1234-4890-abcd-ef0123456789";
const P1: &str = "11111111-1111-4111-8111-111111111111";
const P2: &str = "22222222-2222-4222-8222-222222222222";
const P3: &str = "33333333-3333-4333-8333-333333333333";

/// A session meta with one slot per interesting state.
fn full_meta() -> String {
    format!(
        "session=hx\nsession_id={AID}\nsession_id_origin=session\nschema=2\nmode=local\n\
         origin=/o\nlayout=vertical\n\
         seat.main=lead\nprofile.main=fable\nagent_bin.main=claude\n\
         harness_session.main={}\nconfig_home.main=/h/.claude-mic\n\
         harness_session_prior.main=codex:{P1},{P2}\n\
         seat.worker.0=colead\nprofile.worker.0=sol\nagent_bin.worker.0=codex\n\
         harness_session.worker.0=pending\nconfig_home.worker.0=implicit:/h/.codex\n\
         config_home_base.worker.0=/h\n\
         seat.spawned.3=nav\nprofile.spawned.3=x%y\nagent_bin.spawned.3=gemini\n\
         harness_session.spawned.3={P3}\nconfig_home.spawned.3=unknown\n",
        CUR.to_ascii_uppercase()
    )
}

/// The exact record `full_meta` must produce: roster order, an uppercase current
/// id lowercased (today's purge reads it through the canonical grammar), a legacy
/// bare prior tagged with the slot's binary, `%` escaped, empty current/priors.
fn full_record() -> Vec<u8> {
    format!(
        "version\t1\narchive_id\t{AID}\n\
         main\tfable\tclaude\tpath:/h/.claude-mic\tmissing\tclaude:{CUR}\tcodex:{P1},claude:{P2}\n\
         worker.0\tsol\tcodex\timplicit:/h/.codex\tpath:/h\t\t\n\
         spawned.3\tx%25y\tgemini\tunknown\tmissing\tgemini:{P3}\t\n"
    )
    .into_bytes()
}

fn conv(tool: &str, id: &str) -> Conversation {
    Conversation {
        tool: tool.to_owned(),
        id: id.to_owned(),
    }
}

#[test]
fn harness_a_full_meta_encodes_to_the_exact_record() {
    let (record, notes) = harness::from_meta(full_meta().as_bytes(), AID);
    assert_eq!(record.archive_id, AID);
    let slots: Vec<&str> = record.slots.iter().map(|s| s.slot.as_str()).collect();
    assert_eq!(slots, ["main", "worker.0", "spawned.3"], "roster order");
    let main = &record.slots[0];
    assert_eq!(main.profile, "fable");
    assert_eq!(main.binary, "claude");
    assert_eq!(
        main.config_home,
        RecordedConfigHome::Path(PathBuf::from("/h/.claude-mic"))
    );
    assert_eq!(main.config_home_base, RecordedConfigHomeBase::Missing);
    assert_eq!(main.current, Some(conv("claude", CUR)));
    assert_eq!(main.priors, [conv("codex", P1), conv("claude", P2)]);
    let worker = &record.slots[1];
    assert_eq!(
        worker.config_home,
        RecordedConfigHome::Implicit(PathBuf::from("/h/.codex"))
    );
    assert_eq!(
        worker.config_home_base,
        RecordedConfigHomeBase::Path(PathBuf::from("/h"))
    );
    assert_eq!(worker.current, None, "pending is no conversation");
    assert_eq!(record.slots[2].config_home, RecordedConfigHome::Unknown);
    assert_eq!(record.slots[2].current, Some(conv("gemini", P3)));
    assert!(
        notes
            .iter()
            .all(|n| !n.contains("main") && !n.contains("spawned.3")),
        "clean slots are not noted: {notes:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&harness::encode(&record)),
        String::from_utf8_lossy(&full_record())
    );
}

#[test]
fn harness_the_exact_record_parses_back_and_re_encodes_byte_identical() {
    let (expected, _) = harness::from_meta(full_meta().as_bytes(), AID);
    let parsed = harness::parse(&full_record(), AID).expect("a canonical record parses");
    assert_eq!(parsed, expected);
    assert_eq!(harness::encode(&parsed), full_record());
    let empty = format!("version\t1\narchive_id\t{AID}\n");
    let none = harness::parse(empty.as_bytes(), AID).expect("a slotless record parses");
    assert!(none.slots.is_empty());
    assert_eq!(harness::encode(&none), empty.as_bytes());
    // `invalid` is a recorded state in its own right, for the home and the base alike.
    let invalid = format!("{empty}worker.2\tp\tclaude\tinvalid\tinvalid\t\t\n");
    let parsed = harness::parse(invalid.as_bytes(), AID).expect("invalid states parse");
    assert_eq!(parsed.slots[0].config_home, RecordedConfigHome::Invalid);
    assert_eq!(
        parsed.slots[0].config_home_base,
        RecordedConfigHomeBase::Invalid
    );
    assert_eq!(harness::encode(&parsed), invalid.as_bytes());
}

/// Field `index` (0-based) of the row whose slot is `slot`.
fn field(bytes: &[u8], slot: &str, index: usize) -> String {
    let text = String::from_utf8(bytes.to_vec()).expect("utf-8 record");
    let row = text
        .lines()
        .find(|line| line.split('\t').next() == Some(slot))
        .unwrap_or_else(|| panic!("no row for {slot} in {text:?}"));
    let fields: Vec<&str> = row.split('\t').collect();
    assert_eq!(fields.len(), 7, "seven fields: {row:?}");
    fields[index].to_owned()
}

#[test]
fn harness_damage_is_noted_never_guessed_and_never_collapsed_to_missing() {
    let six: Vec<String> = (0..6)
        .map(|n| format!("codex:{n}{n}{n}{n}{n}{n}{n}{n}-0000-4000-8000-00000000000{n}"))
        .collect();
    let meta = format!(
        "session=hd\nsession_id={AID}\nschema=2\n\
         seat.main=a\nagent_bin.main=claude\nharness_session.main=not-a-uuid\n\
         config_home.main=relative/path\n\
         seat.worker.0=b\nagent_bin.worker.0=cl\taude\nharness_session.worker.0={CUR}\n\
         config_home.worker.0=absent\n\
         seat.worker.1=c\nagent_bin.worker.1=claude\n\
         harness_session_prior.worker.1=codex:{},bogus,{},agy:{P3}\n\
         config_home.worker.1=/h/one\nconfig_home.worker.1=/h/two\n\
         seat.spawned.0=d\nagent_bin.spawned.0=codex\nharness_session_prior.spawned.0={}\n\
         seat.spawned.1=e\nagent_bin.spawned.1=codex\nconfig_home.spawned.1=implicit:/h/.codex\n\
         config_home_base.spawned.1=/h\nconfig_home_base.spawned.1=/g\n",
        P1.to_ascii_uppercase(),
        P2.to_ascii_uppercase(),
        six.join(",")
    );
    let (record, notes) = harness::from_meta(meta.as_bytes(), AID);
    let bytes = harness::encode(&record);
    let count = |slot: &str| notes.iter().filter(|n| n.contains(slot)).count();

    // An id that is not a UUID names nothing.
    assert_eq!(record.slots[0].current, None);
    assert!(count("main") >= 1, "{notes:?}");
    // A binary outside the tool grammar tags nothing, and stays inert escaped evidence.
    assert_eq!(record.slots[1].current, None);
    assert!(count("worker.0") >= 1, "{notes:?}");
    assert_eq!(field(&bytes, "worker.0", 2), "cl%09aude");
    // The meta reader lowercases predecessors; a garbage element is omitted and
    // noted, a bare id takes the slot's binary, a foreign tag survives.
    assert_eq!(
        record.slots[2].priors,
        [conv("codex", P1), conv("claude", P2), conv("agy", P3)]
    );
    assert!(count("worker.1") >= 1, "{notes:?}");
    // More than four predecessors: the newest four kept, oldest first (the meta
    // writer evicts the oldest past four), every omitted one noted.
    let kept: Vec<Conversation> = six[2..]
        .iter()
        .map(|p| conv("codex", p.trim_start_matches("codex:")))
        .collect();
    assert_eq!(record.slots[3].priors, kept);
    assert!(count("spawned.0") >= 2, "{notes:?}");
    // Six home states, never collapsed: invalid, absent, invalid (duplicate), missing.
    assert_eq!(field(&bytes, "main", 3), "invalid");
    // Three base states, never collapsed: a duplicate base is invalid, not missing.
    assert_eq!(
        record.slots[4].config_home_base,
        RecordedConfigHomeBase::Invalid
    );
    assert_eq!(field(&bytes, "spawned.1", 4), "invalid");
    assert!(count("spawned.1") >= 1, "{notes:?}");
    assert_eq!(field(&bytes, "worker.0", 3), "absent");
    assert_eq!(field(&bytes, "worker.1", 3), "invalid");
    assert_eq!(field(&bytes, "spawned.0", 3), "missing");
    // Whatever was damaged, the record it wrote parses back to itself.
    assert_eq!(
        harness::parse(&bytes, AID).expect("own output parses"),
        record
    );
}

#[test]
fn harness_parse_refuses_every_malformed_shape() {
    let good = String::from_utf8(full_record()).unwrap();
    let r = |from: &str, to: &str| {
        assert!(good.contains(from), "fixture holds {from:?}");
        good.replacen(from, to, 1).into_bytes()
    };
    let head = format!("version\t1\narchive_id\t{AID}\n");
    let swapped = format!("archive_id\t{AID}\nversion\t1\n");
    let home = "path:/h/.claude-mic";
    let base = "\tmissing\tclaude:";
    let cur = format!("claude:{CUR}");
    let pri = format!("codex:{P1},claude:{P2}");
    let pad = (1 << 20) - good.len() + "fable".len();
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("version 2", r("version\t1", "version\t2")),
        ("version 01", r("version\t1", "version\t01")),
        ("uppercase archive id", r(AID, &AID.to_ascii_uppercase())),
        ("no final newline", good.trim_end().as_bytes().to_vec()),
        ("CRLF", good.replace('\n', "\r\n").into_bytes()),
        ("blank line", r("\nmain\t", "\n\nmain\t")),
        ("unknown header", r("\nmain\t", "\nextra\tx\nmain\t")),
        ("header order", r(&head, &swapped)),
        ("six fields", r(&format!("\t{pri}\n"), "\n")),
        (
            "eight fields",
            r(&format!("\t{pri}\n"), &format!("\t{pri}\t\n")),
        ),
        ("slot name", r("\nmain\t", "\nlead\t")),
        ("slot leading zero", r("worker.0\t", "worker.00\t")),
        ("slot no number", r("worker.0\t", "worker.\t")),
        ("duplicate slot", r("worker.0\t", "main\t")),
        ("relative path home", r(home, "path:relative")),
        ("empty path home", r(home, "path:")),
        ("relative implicit home", r(home, "implicit:rel")),
        ("unknown home token", r(home, "maybe")),
        ("home token case", r(home, "Missing")),
        ("bare home path", r(home, "/h/x")),
        ("control in path", r(home, "path:/h/%09x")),
        ("base absent", r(base, "\tabsent\tclaude:")),
        ("base implicit", r(base, "\timplicit:/h\tclaude:")),
        ("current not uuid", r(&cur, "claude:nope")),
        (
            "current uppercase",
            r(&cur, &format!("claude:{}", CUR.to_ascii_uppercase())),
        ),
        ("current untagged", r(&cur, CUR)),
        ("current empty tool", r(&cur, &format!(":{CUR}"))),
        ("tool grammar", r(&cur, &format!("cl aude:{CUR}"))),
        (
            "tool too long",
            r(&cur, &format!("{}:{CUR}", "t".repeat(65))),
        ),
        (
            "five priors",
            r(&pri, &vec![format!("codex:{P1}"); 5].join(",")),
        ),
        ("trailing comma", r(&pri, &format!("{pri},"))),
        ("empty prior", r(&pri, &format!("codex:{P1},,claude:{P2}"))),
        ("bare prior", r(&pri, P1)),
        ("short escape", r("x%25y", "x%2y")),
        ("lowercase escape", r("fable", "fa%1bble")),
        ("non-hex escape", r("fable", "fa%zzble")),
        ("needless escape", r("fable", "f%41ble")),
        ("raw control", r("fable", "fa\u{1}ble")),
        ("1 MiB + 1", r("fable", &"a".repeat(pad + 1))),
    ];
    let mut bad_utf8 = full_record();
    let at = bad_utf8.windows(5).position(|w| w == b"fable").unwrap();
    bad_utf8[at] = 0xff;
    cases.push(("invalid utf-8", bad_utf8));
    let rows = |n: usize| {
        let mut text = head.clone();
        for i in 0..n {
            let _ = writeln!(text, "spawned.{i}\t\t\tmissing\tmissing\t\t");
        }
        text
    };
    cases.push(("1025 slots", rows(1025).into_bytes()));
    for (what, bytes) in &cases {
        assert!(
            harness::parse(bytes, AID).is_err(),
            "{what}: a malformed record must be refused"
        );
    }
    assert!(
        harness::parse(&full_record(), "a1b2c3d4-0000-4000-8000-000000000002").is_err(),
        "an archive_id that is not the expected archive is refused"
    );
    let limit = rows(1024);
    let parsed = harness::parse(limit.as_bytes(), AID).expect("1024 slots is the bound");
    assert_eq!(parsed.slots.len(), 1024);
    assert_eq!(harness::encode(&parsed), limit.as_bytes());
    let exact = r("fable", &"a".repeat(pad));
    assert_eq!(exact.len(), 1 << 20);
    let parsed = harness::parse(&exact, AID).expect("exactly 1 MiB is the bound");
    assert_eq!(harness::encode(&parsed), exact);
}

// ---- black box: the publisher writes it, --from never reads it -----------------

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(tag: &str) -> Scratch {
    Scratch(super::cli::OwnedScratch::root("harness", tag).keep())
}

/// `<home>/sessions/<name>` holding `meta` and empty ledgers.
fn session(home: &Path, name: &str, meta: &str) -> PathBuf {
    let dir = home.join("sessions").join(name);
    std::fs::create_dir_all(dir.join("messages")).unwrap();
    std::fs::create_dir_all(home.join("archive")).unwrap();
    std::fs::write(dir.join("meta"), meta).unwrap();
    std::fs::write(dir.join("memo.tsv"), "").unwrap();
    std::fs::write(dir.join("events.jsonl"), "").unwrap();
    dir
}

fn core(home: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = super::cli::ae();
    cmd.env("AE_HOME", home);
    for arg in args {
        cmd.arg(arg);
    }
    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    super::cli::bounded(child, Duration::from_secs(10)).expect("returned")
}

fn publish(home: &Path, dir: &Path) {
    let d = dir.to_str().unwrap();
    let out = core(
        home,
        &["_archive-publish", d, "not-managed", "-", "-", "-", AT],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "publish: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn from_preflight(home: &Path) -> std::process::Output {
    core(
        home,
        &[
            "_archive-from-preflight",
            home.join("archive").to_str().unwrap(),
            AID,
        ],
    )
}

fn mode_of(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn harness_end_publish_writes_the_record_reason_and_work_dir_never_into_meta() {
    let s = scratch("pub");
    let meta = format!("{}work_dir=/w/hx\n", full_meta());
    let dir = session(&s.0, "hx", &meta);
    publish(&s.0, &dir);
    let target = s.0.join("archive").join(AID);
    let record = target.join(harness::FILE);
    let node = std::fs::symlink_metadata(&record).expect("harness.tsv published");
    assert!(node.is_file() && !node.file_type().is_symlink());
    assert_eq!(mode_of(&record), 0o600, "inert 0600, no executable bit");
    assert_eq!(std::fs::read(&record).unwrap(), full_record());
    let archived = std::fs::read_to_string(target.join("meta")).unwrap();
    let reasons: Vec<&str> = archived
        .lines()
        .filter(|l| l.starts_with("reason="))
        .collect();
    assert_eq!(reasons, ["reason=end"], "{archived}");
    assert!(
        archived.lines().any(|l| l == "source_work_dir=/w/hx"),
        "{archived}"
    );
    for key in ["harness_session", "config_home", "profile."] {
        assert!(
            !archived.contains(key),
            "{key} never reaches archive meta: {archived}"
        );
    }
    let s2 = scratch("pub-nowd");
    let dir2 = session(&s2.0, "hx", &full_meta());
    publish(&s2.0, &dir2);
    let archived = std::fs::read_to_string(s2.0.join("archive").join(AID).join("meta")).unwrap();
    assert!(
        archived.lines().any(|l| l == "source_work_dir=-"),
        "{archived}"
    );
}

#[test]
fn harness_a_record_past_the_reader_bound_is_still_published_whole() {
    // Lead ruling I9: no new size bound makes End refuse. The publisher writes the
    // whole record; the bounded reader refusing it is purge's named gap (P2).
    let s = scratch("big");
    let big = "p".repeat(1 << 20);
    let meta = full_meta().replacen("profile.main=fable", &format!("profile.main={big}"), 1);
    let dir = session(&s.0, "hx", &meta);
    publish(&s.0, &dir);
    let bytes = std::fs::read(s.0.join("archive").join(AID).join(harness::FILE)).unwrap();
    let expected = String::from_utf8(full_record())
        .unwrap()
        .replacen("fable", &big, 1);
    assert!(bytes.len() > 1 << 20);
    assert!(
        bytes == expected.as_bytes(),
        "the whole record, nothing clipped"
    );
    assert!(
        harness::parse(&bytes, AID).is_err(),
        "the reader keeps its bound"
    );
}

#[test]
fn harness_from_never_opens_the_record_and_legacy_archives_stay_inheritable() {
    let s = scratch("from");
    let dir = session(&s.0, "hx", &full_meta());
    publish(&s.0, &dir);
    let record = s.0.join("archive").join(AID).join(harness::FILE);
    std::fs::write(&record, b"\xff\x00 not a record\n").unwrap();
    let out = from_preflight(&s.0);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--from does not parse the record: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).starts_with(AID));
    std::fs::remove_file(&record).unwrap();
    let code = from_preflight(&s.0).status.code();
    assert_eq!(code, Some(0), "legacy archive");
}

#[test]
fn harness_a_record_that_is_not_an_inert_regular_file_fails_validation() {
    let s = scratch("node");
    let dir = session(&s.0, "hx", &full_meta());
    publish(&s.0, &dir);
    let record = s.0.join("archive").join(AID).join(harness::FILE);
    let outside = s.0.join("outside.tsv");
    std::fs::write(&outside, full_record()).unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();
    let socket = s.0.join("sk");
    for what in ["mode 644", "executable", "symlink", "directory", "socket"] {
        let _ = std::fs::remove_file(&record);
        let _ = std::fs::remove_dir(&record);
        match what {
            "symlink" => std::os::unix::fs::symlink(&outside, &record).unwrap(),
            "directory" => std::fs::create_dir(&record).unwrap(),
            "socket" => {
                drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
                std::fs::rename(&socket, &record).unwrap();
            }
            _ => {
                std::fs::write(&record, full_record()).unwrap();
                let mode = if what == "executable" { 0o700 } else { 0o644 };
                std::fs::set_permissions(&record, std::fs::Permissions::from_mode(mode)).unwrap();
            }
        }
        let code = from_preflight(&s.0).status.code();
        assert_eq!(code, Some(1), "{what} refused");
    }
}
