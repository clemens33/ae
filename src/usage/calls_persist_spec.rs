//! Persistence acceptance from P1–P7. Synthetic sources and hand counts.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned synthetic fixture IO and acceptance assertions"
)]

use super::{LSTATS, Reader, STEP, Unit, UnitKind};
use crate::stats_store::disk;
use crate::tool_stats::{Ask, Caps, Cols, Filter, GapKind, Page, Scope, View};
use crate::tool_stats_fixture::{self as f, Fixture};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn ask(page: Page) -> Ask {
    Ask {
        page,
        scope: Scope::All,
        filter: Filter::All,
    }
}

fn open(root: &Fixture) -> Reader {
    Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE)
}

fn round(reader: &mut Reader) -> Vec<Unit> {
    reader.round();
    let mut units = Vec::new();
    for _ in 0..30_000 {
        let Some(unit) = reader.step() else {
            assert!(!reader.busy());
            return units;
        };
        assert!(unit.bytes <= STEP, "actual native bytes: {unit:?}");
        assert!(unit.lstats <= LSTATS, "actual stat calls: {unit:?}");
        units.push(unit);
    }
    panic!("finite persistence fixture did not settle");
}

fn fixture(bytes: &[u8]) -> (Fixture, PathBuf) {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        &f::event("ordinary", "epoch fixture"),
    );
    prove_epoch(&root);
    let path = root.claude("store-a", f::C, bytes);
    (root, path)
}

// S5 alone owns the payload and its epoch. Exercise its real first publication.
fn prove_epoch(root: &Fixture) {
    let mut maintainer = crate::stats_store::maint::Maintainer::new(
        &root.0,
        disk::LIMITS,
        Duration::from_millis(100),
    );
    for _ in 0..100 {
        let _ = maintainer.step();
        if root.0.join("stats/epoch").is_file() {
            return;
        }
    }
    panic!("real S5 publication must establish an epoch before Tools persistence");
}

fn counts(view: &View) -> (u64, u64) {
    match view.totals {
        Cols::Tool(cols) => (cols.attempts, cols.returned),
        other => panic!("Tools columns required: {other:?}"),
    }
}

fn expect(reader: &Reader, attempts: u64, returned: u64) {
    assert_eq!(
        counts(&reader.view(&ask(Page::Tools))),
        (attempts, returned)
    );
}

fn append(path: &Path, bytes: &[u8]) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .expect("existing source")
        .write_all(bytes)
        .expect("actual append without replacing the inode");
}

fn native_bytes(units: &[Unit]) -> u64 {
    units
        .iter()
        .filter(|unit| matches!(unit.kind, UnitKind::Read | UnitKind::Verify))
        .map(|unit| unit.bytes)
        .sum()
}

fn cold_equal(bytes: &[u8], reader: &Reader, attempts: u64, returned: u64) {
    // An independent root contains source bytes only, never copied stats files.
    let (cold, _) = fixture(bytes);
    let mut rebuilt = open(&cold);
    round(&mut rebuilt);
    expect(reader, attempts, returned);
    expect(&rebuilt, attempts, returned);
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        assert_eq!(
            reader.view(&ask(page)),
            rebuilt.view(&ask(page)),
            "persisted == cold {page:?}"
        );
    }
}

// Lead ruling 2026-11-09: damage history has no counterpart in a cold root.
// Remove exactly its one asserted gap; compare every other View field intact.
fn cold_equal_after_damage(bytes: &[u8], reader: &Reader, attempts: u64, returned: u64) {
    let (cold, _) = fixture(bytes);
    let mut rebuilt = open(&cold);
    round(&mut rebuilt);
    expect(reader, attempts, returned);
    expect(&rebuilt, attempts, returned);
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        let mut view = reader.view(&ask(page));
        let damage: Vec<usize> = view
            .gaps
            .iter()
            .enumerate()
            .filter_map(|(index, gap)| (gap.kind == GapKind::RebuiltFromSource).then_some(index))
            .collect();
        assert_eq!(damage.len(), 1, "exactly one damage gap on {page:?}");
        let gap = view.gaps.remove(damage[0]);
        assert_eq!(gap.count, 1, "one rebuilt conversation on {page:?}");
        assert_eq!(gap.conversation, None, "conversation-wide damage total");
        assert_eq!(
            gap.line(),
            "rebuilt from source: 1 conversations (counts of deleted sources there are lost)"
        );
        assert_eq!(view, rebuilt.view(&ask(page)), "persisted == cold {page:?}");
    }
}

fn first() -> String {
    f::call(
        "first-native-secret-id",
        "first-message-secret",
        "Read",
        "model-a",
        "{}",
    )
}

fn large() -> String {
    let mut bytes = first();
    // Many short ignored records make the old prefix exceed two witness blocks.
    let padding = "{\"type\":\"unrelated\",\"text\":\"ordinary padding\"}\n";
    while bytes.len() < (3 << 20) + 8192 {
        bytes.push_str(padding);
    }
    assert_ne!(
        bytes.len() % (1 << 20),
        0,
        "fixture has a partial final block"
    );
    bytes
}

fn files(directory: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if !directory.exists() {
        return result;
    }
    for entry in fs::read_dir(directory).expect("owned stats directory") {
        let path = entry.expect("fixture entry").path();
        if path.is_dir() {
            result.extend(files(&path));
        } else {
            result.push(path);
        }
    }
    result.sort();
    result
}

fn projection_files(root: &Fixture) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for entry in fs::read_dir(&root.0).expect("owned fixture root") {
        let entry = entry.expect("fixture entry");
        if entry.file_name().to_string_lossy().starts_with("stats") {
            if entry.path().is_dir() {
                result.extend(files(&entry.path()));
            } else {
                result.push(entry.path());
            }
        }
    }
    result
}

// Q1b permits only roots ae recorded in this fixture's meta. Mask only
// their JSON string contents in association documents, never equal bytes
// in another field or another file. Keep all offsets and other bytes intact.
fn mask_recorded_roots(bytes: &[u8], recorded: &BTreeSet<String>) -> Option<Vec<u8>> {
    use crate::json::Value;
    let text = std::str::from_utf8(bytes).ok()?;
    let mut masked = bytes.to_vec();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let Value::Obj(fields) = crate::json::parse(line.trim_end_matches('\n')).ok()? else {
            return None;
        };
        for (key, value) in fields {
            let roots = match (key.as_str(), &value) {
                ("store", Value::Null) => Vec::new(),
                ("store", Value::Str(root)) => vec![root.as_str()],
                ("homes" | "dirs", Value::Arr(roots)) => roots
                    .iter()
                    .map(Value::as_str)
                    .collect::<Option<Vec<_>>>()?,
                ("store" | "homes" | "dirs", _) => return None,
                _ => continue,
            };
            let encoded_value = value.render();
            let prefix = format!("{}:", Value::str(&key).render());
            let field = format!("{prefix}{encoded_value}");
            let start = offset + line.find(&field)? + prefix.len();
            let mut cursor = 0;
            for root in roots {
                if !recorded.contains(root) {
                    return None;
                }
                let encoded = Value::str(root).render();
                let at = cursor + encoded_value[cursor..].find(&encoded)?;
                // Preserve the quotes and array punctuation too.
                masked[start + at + 1..start + at + encoded.len() - 1].fill(b' ');
                cursor = at + encoded.len();
            }
        }
        offset += line.len();
    }
    Some(masked)
}

fn projection_bytes(root: &Fixture, path: &Path) -> Vec<u8> {
    let bytes = fs::read(path).expect("persisted fixture projection");
    if path.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("tools")) {
        return bytes;
    }
    match path.file_name().and_then(|name| name.to_str()) {
        Some("assoc" | "assoc.tmp") => {
            crate::tool_stats::doc::parse_assoc(&bytes).expect("valid association document");
        }
        Some("sources" | "sources.tmp") => {
            crate::tool_stats::doc::parse_sources(&bytes).expect("valid association sources");
        }
        _ => return bytes,
    }
    let meta =
        fs::read_to_string(root.0.join("sessions/alpha/meta")).expect("fixture-recorded meta");
    let recorded: BTreeSet<_> = meta
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| key.starts_with("config_home.") || key.starts_with("work_dir."))
        .map(|(_, value)| value.to_owned())
        .collect();
    assert!(!recorded.is_empty(), "positive recorded-root control");
    mask_recorded_roots(&bytes, &recorded)
        .expect("every association root must equal a fixture-recorded meta root")
}

#[test]
fn q1b_root_mask_preserves_non_root_bytes_and_refuses_unrecorded_roots() {
    let root = format!("/fixture/{}", f::PRIVATE);
    let recorded = BTreeSet::from([root.clone()]);
    let bytes = format!(
        "{{\"store\":\"{root}\",\"homes\":[\"{root}\"],\"dirs\":[\"{root}\"],\"other\":\"{root}\"}}\n"
    );
    let spaces = " ".repeat(root.len());
    let expected = format!(
        "{{\"store\":\"{spaces}\",\"homes\":[\"{spaces}\"],\"dirs\":[\"{spaces}\"],\"other\":\"{root}\"}}\n"
    );
    let masked = mask_recorded_roots(bytes.as_bytes(), &recorded).expect("recorded roots only");
    assert_eq!(
        masked,
        expected.as_bytes(),
        "all other bytes stay identical"
    );
    assert!(
        masked
            .windows(f::PRIVATE.len())
            .any(|part| part == f::PRIVATE.as_bytes()),
        "an equal root in a non-root field must still fail the sentinel scan"
    );
    for field in ["store", "homes", "dirs"] {
        let value = if field == "store" {
            format!("\"{root}/unrecorded\"")
        } else {
            format!("[\"{root}/unrecorded\"]")
        };
        let unrecorded = format!("{{\"{field}\":{value}}}\n");
        assert!(mask_recorded_roots(unrecorded.as_bytes(), &recorded).is_none());
    }
}

#[test]
fn p1_restart_restores_counts_and_unchanged_sources_read_zero_bytes() {
    let bytes = first() + &f::result("first-native-secret-id", false, None);
    let (root, _) = fixture(bytes.as_bytes());
    let mut reader = open(&root);
    let units = round(&mut reader);
    assert!(native_bytes(&units) > 0, "positive read-observer control");
    cold_equal(bytes.as_bytes(), &reader, 1, 1);
    assert!(
        !files(&root.0.join("stats/tools/conv")).is_empty(),
        "durable conversation state exists"
    );
    drop(reader);
    let mut reopened = open(&root);
    let units = round(&mut reopened);
    assert_eq!(native_bytes(&units), 0, "restart must not repeat backfill");
    assert!(
        !units
            .iter()
            .any(|unit| unit.kind == UnitKind::Meta && unit.bytes > 0),
        "unchanged recorded meta is cached too"
    );
    cold_equal(bytes.as_bytes(), &reopened, 1, 1);
}

#[test]
fn p3_append_resumes_after_restart_and_late_result_inherits_the_old_attempt() {
    let mut bytes = large();
    let (root, path) = fixture(bytes.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 1, 0);
    drop(reader);
    let added = f::call("second", "second", "Bash", "model-b", "{}")
        + &f::result("first-native-secret-id", false, None);
    append(&path, added.as_bytes());
    bytes.push_str(&added);
    let mut reader = open(&root);
    let units = round(&mut reader);
    let reads: u64 = units
        .iter()
        .filter(|unit| unit.kind == UnitKind::Read)
        .map(|unit| unit.bytes)
        .sum();
    assert_eq!(reads, u64::try_from(added.len()).expect("append length"));
    assert!(
        native_bytes(&units) <= u64::try_from(added.len()).expect("append length") + (1 << 20),
        "only the final partial block is witnessed"
    );
    cold_equal(bytes.as_bytes(), &reader, 2, 1);
    let view = reader.view(&ask(Page::Tools));
    let original: Vec<_> = view.detail.iter().filter(|row| matches!(&row.item, crate::tool_stats::Item::Tool(crate::tool_stats::Ident::Name(name)) if name == "Read")).collect();
    assert_eq!(original.len(), 1);
    assert_eq!(
        original[0].model,
        crate::stats::Cell::Value("model-a".into())
    );
}

#[test]
fn p3_partial_record_survives_publication_and_restart_without_raw_tail_storage() {
    let second = f::call(
        "second",
        "second",
        "Bash",
        "model-b",
        &format!("{{\"secret\":\"{}\"}}", f::PRIVATE),
    );
    let split = second.len() - 4;
    let mut bytes = first() + &second[..split];
    let (root, path) = fixture(bytes.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 1, 0);
    for path in projection_files(&root) {
        assert!(
            !projection_bytes(&root, &path)
                .windows(f::PRIVATE.len())
                .any(|part| part == f::PRIVATE.as_bytes()),
            "raw partial JSON escaped to {path:?}"
        );
    }
    drop(reader);
    append(&path, &second.as_bytes()[split..]);
    bytes.push_str(&second[split..]);
    let mut reader = open(&root);
    let units = round(&mut reader);
    assert!(
        native_bytes(&units) <= u64::try_from(second.len()).expect("record length") + (1 << 20)
    );
    cold_equal(bytes.as_bytes(), &reader, 2, 0);
    drop(reader);
    let mut reader = open(&root);
    assert_eq!(native_bytes(&round(&mut reader)), 0);
    expect(&reader, 2, 0);
}

#[test]
fn p3_overlong_tail_skipping_survives_restart_and_counts_the_gap_once() {
    let mut bytes = first().into_bytes();
    bytes.extend(vec![b'x'; (1 << 20) + 17]);
    let (root, path) = fixture(&bytes);
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 1, 0);
    let over = |reader: &Reader| {
        reader
            .view(&ask(Page::Tools))
            .gaps
            .iter()
            .filter(|line| line.kind == GapKind::OverCap)
            .map(|line| line.count)
            .sum::<u64>()
    };
    assert_eq!(over(&reader), 1);
    drop(reader);
    let mut reader = open(&root);
    assert_eq!(native_bytes(&round(&mut reader)), 0);
    assert_eq!(over(&reader), 1);
    drop(reader);
    let added = "xxxxx\n".to_owned() + &f::call("after", "after", "Bash", "model-b", "{}");
    append(&path, added.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 2, 0);
    assert_eq!(
        over(&reader),
        1,
        "persisted skipping suppresses another gap for the same line"
    );
}

#[test]
fn p2_recorded_meta_edit_relabels_persisted_counts_without_a_transcript_reread() {
    let (root, _) = fixture(first().as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    assert_eq!(
        reader.view(&ask(Page::Tools)).detail[0].client,
        crate::stats::Cell::Value("work".into())
    );
    drop(reader);
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("personal"), "store-a");
    Fixture::rewrite(&root.0.join("sessions/alpha/meta"), meta.as_bytes());
    let mut reader = open(&root);
    assert_eq!(native_bytes(&round(&mut reader)), 0);
    expect(&reader, 1, 0);
    assert_eq!(
        reader.view(&ask(Page::Tools)).detail[0].client,
        crate::stats::Cell::Value("personal".into())
    );
    assert_eq!(
        reader.view(&ask(Page::Tools)).detail[0].model,
        crate::stats::Cell::Value("model-a".into())
    );
}

#[test]
fn p3_distant_same_length_rewrite_rebuilds_then_truncation_and_identity_change_replace_counts() {
    let bytes = large();
    let (root, path) = fixture(bytes.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    drop(reader);
    let changed = bytes.replacen("Read", "Bash", 1);
    assert_eq!(changed.len(), bytes.len());
    assert_eq!(
        &changed[changed.len() - (1 << 20)..],
        &bytes[bytes.len() - (1 << 20)..]
    );
    Fixture::rewrite(&path, changed.as_bytes());
    let mut reader = open(&root);
    let units = round(&mut reader);
    assert!(
        units
            .iter()
            .any(|unit| unit.kind == UnitKind::Verify && unit.bytes > 0)
    );
    cold_equal(changed.as_bytes(), &reader, 1, 0);
    drop(reader);

    let short = f::call("short", "short", "Write", "model-c", "{}");
    Fixture::rewrite(&path, short.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    cold_equal(short.as_bytes(), &reader, 1, 0);
    drop(reader);

    let replacement = first() + &f::call("replacement", "replacement", "Bash", "model-b", "{}");
    let alternate = path.with_extension("replacement");
    Fixture::rewrite(&alternate, replacement.as_bytes());
    fs::rename(alternate, &path).expect("new inode at the same path");
    let mut reader = open(&root);
    round(&mut reader);
    cold_equal(replacement.as_bytes(), &reader, 2, 0);
}

#[test]
fn p6_source_gone_keeps_counts_after_restart_and_reset_rebuilds_only_existing_sources() {
    let bytes = first() + &f::result("first-native-secret-id", false, None);
    let (root, path) = fixture(bytes.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 1, 1);
    drop(reader);
    fs::remove_file(path).expect("source deleted after durable publication");
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 1, 1);
    let view = reader.view(&ask(Page::Tools));
    assert!(view.partial());
    assert_eq!(
        view.gaps
            .iter()
            .filter(|line| line.kind == GapKind::SourceGone)
            .map(|line| line.count)
            .sum::<u64>(),
        1
    );
    drop(reader);
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 1, 1);
    drop(reader);
    assert!(!matches!(
        disk::reset(&root.0, Duration::ZERO).expect("actual statistics reset"),
        disk::Reset::Busy
    ));
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 0, 0);
}

#[test]
fn p3_edit_plus_growth_outside_the_witness_is_the_documented_append_only_boundary() {
    let bytes = large();
    let (root, path) = fixture(bytes.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    drop(reader);
    let added = f::call("new", "new", "Write", "model-b", "{}");
    let changed = bytes.replacen("Read", "Bash", 1) + &added;
    Fixture::rewrite(&path, changed.as_bytes());
    let mut reader = open(&root);
    let units = round(&mut reader);
    assert!(native_bytes(&units) <= u64::try_from(added.len()).expect("append length") + (1 << 20));
    expect(&reader, 2, 0);
    // This input violates the accepted append-only contract. Do not claim
    // warm == fresh: the old full block was not re-observed.
    let warm = format!("{:?}", reader.view(&ask(Page::Tools)).rows);
    assert!(warm.contains("Read") && warm.contains("Write"));
    assert!(!warm.contains("Bash"));
    let (fresh, _) = fixture(changed.as_bytes());
    let mut cold = open(&fresh);
    round(&mut cold);
    let rows = format!("{:?}", cold.view(&ask(Page::Tools)).rows);
    assert!(rows.contains("Bash") && rows.contains("Write"));
    assert!(
        !rows.contains("Read"),
        "cold rebuild sees the replaced call only"
    );
}

#[test]
fn p2_persisted_projection_contains_only_allowlisted_words_and_opaque_keys() {
    let directory = format!("/opaque/{}/resource", f::PRIVATE);
    let bytes = f::call(
        "secret-native-id",
        "secret-message-id",
        "Skill",
        "model-a",
        &format!(
            "{{\"skill\":\"allowed-skill\",\"args\":\"{}\"}}",
            f::PRIVATE
        ),
    ) + &f::result("secret-native-id", false, Some(true))
        + &f::base("secret-native-id", &directory)
        + &f::call(
            "read-secret-id",
            "read-message-id",
            "Read",
            "model-a",
            &format!("{{\"file_path\":\"{directory}/guide.md\"}}"),
        )
        + &f::result("read-secret-id", false, None)
        + "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"DUMMY_CREDENTIAL_PRIVATE\"}]}}\n";
    let (root, _) = fixture(bytes.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 2, 2);
    let paths = projection_files(&root);
    assert!(
        !paths.is_empty(),
        "privacy must examine actual persisted bytes"
    );
    let mut found_identity = false;
    for path in paths {
        let bytes = projection_bytes(&root, &path);
        found_identity |= bytes
            .windows(b"allowed-skill".len())
            .any(|part| part == b"allowed-skill");
        for private in [
            f::PRIVATE,
            "secret-native-id",
            "secret-message-id",
            "read-secret-id",
            "read-message-id",
            "DUMMY_CREDENTIAL_PRIVATE",
        ] {
            assert!(
                !bytes
                    .windows(private.len())
                    .any(|part| part == private.as_bytes()),
                "private bytes persisted: {private}"
            );
        }
    }
    assert!(
        found_identity,
        "positive control: an allowlisted identity is persisted"
    );
}

#[test]
fn p1_future_conversation_format_rebuilds_from_source_instead_of_reusing_stale_counts() {
    let bytes = large();
    let (root, path) = fixture(bytes.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    drop(reader);
    let documents = files(&root.0.join("stats/tools/conv"));
    assert_eq!(documents.len(), 1, "one authoritative conversation");
    let encoded = fs::read(&documents[0]).expect("persisted conversation");
    let lf = encoded
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("head LF");
    let mut head = crate::json::parse(std::str::from_utf8(&encoded[..lf]).expect("head UTF-8"))
        .expect("head JSON");
    let crate::json::Value::Obj(fields) = &mut head else {
        panic!("object head")
    };
    let (_, value) = fields
        .iter_mut()
        .find(|(key, _)| key == "tools_conv")
        .expect("conversation version");
    *value = crate::json::Value::Num(2);
    Fixture::rewrite(
        &documents[0],
        &[head.render().as_bytes(), &encoded[lf..]].concat(),
    );
    let added = f::call("second", "second", "Bash", "model-b", "{}");
    append(&path, added.as_bytes());
    let current = bytes + &added;
    let mut reader = open(&root);
    let units = round(&mut reader);
    let read: u64 = units
        .iter()
        .filter(|unit| unit.kind == UnitKind::Read)
        .map(|unit| unit.bytes)
        .sum();
    assert!(
        read >= u64::try_from(current.len()).expect("whole-source rebuild size"),
        "wrong version forces a real source rebuild"
    );
    cold_equal_after_damage(current.as_bytes(), &reader, 2, 0);
}

fn load_skill(id: &str, name: &str, directory: &str) -> String {
    f::call(
        id,
        id,
        "Skill",
        "model-a",
        &format!("{{\"skill\":\"{name}\"}}"),
    ) + &f::base(id, directory)
        + &f::result(id, false, Some(true))
}

fn successful_read(id: &str, path: &str) -> String {
    f::call(
        id,
        id,
        "Read",
        "model-a",
        &format!("{{\"file_path\":\"{path}\"}}"),
    ) + &f::result(id, false, None)
}

fn skill_reads(reader: &Reader) -> u64 {
    match reader.view(&ask(Page::Skills)).totals {
        Cols::Skill(cols) => cols.reads,
        other => panic!("Skills columns: {other:?}"),
    }
}

#[test]
fn o2_fallback_drops_only_unproven_non_skills_paths_visibly_and_keeps_proved_other_layouts() {
    let mut bytes = load_skill("proved", "proved", "/opaque/proved")
        + &successful_read("proved-read", "/opaque/proved/guide.md");
    // "skills-adjacent" is not a literal "skills" path component.
    for id in 0..64 {
        bytes.push_str(&successful_read(
            &format!("pending-{id}"),
            &format!(
                "/opaque/skills-adjacent/pending/{id}/{}guide.md",
                "d/".repeat(100)
            ),
        ));
    }
    bytes.push_str(&successful_read(
        "keep-pending",
        "/opaque/skills/kept/guide.md",
    ));
    let proof = load_skill(
        "late-outside",
        "late-outside",
        "/opaque/skills-adjacent/pending",
    ) + &load_skill("late-kept", "late-kept", "/opaque/skills/kept");
    let (roomy, roomy_path) = fixture(bytes.as_bytes());
    let mut exact = open(&roomy);
    round(&mut exact);
    assert_eq!(skill_reads(&exact), 1);
    append(&roomy_path, proof.as_bytes());
    round(&mut exact);
    expect(&exact, 69, 69);
    assert_eq!(
        skill_reads(&exact),
        66,
        "positive control: all pending paths later prove"
    );
    assert!(
        !exact
            .view(&ask(Page::Skills))
            .gaps
            .iter()
            .any(|line| line.kind == GapKind::UnprovenReadsDropped)
    );

    let mut saw_fallback = false;
    for power in 14..=21 {
        let (root, path) = fixture(bytes.as_bytes());
        let mut reader = Reader::new(
            root.0.clone(),
            Some(root.home()),
            Caps {
                conv_cap: 1 << power,
                ..Caps::SLICE
            },
        );
        round(&mut reader);
        let view = reader.view(&ask(Page::Skills));
        let dropped: u64 = view
            .gaps
            .iter()
            .filter(|line| line.kind == GapKind::UnprovenReadsDropped)
            .map(|line| line.count)
            .sum();
        if dropped == 0 || view.gaps.iter().any(|line| line.kind == GapKind::OverCap) {
            continue;
        }
        assert_eq!(
            dropped, 64,
            "only the 64 unproven non-skills Reads are dropped"
        );
        assert!(view.partial());
        expect(&reader, 67, 67);
        assert_eq!(
            skill_reads(&reader),
            1,
            "already proved Other-layout Read survives"
        );
        drop(reader);
        // Plenty of publication room now; the lost evidence stays visibly lost.
        append(&path, proof.as_bytes());
        let mut reader = open(&root);
        round(&mut reader);
        expect(&reader, 69, 69);
        assert_eq!(
            skill_reads(&reader),
            2,
            "proved Other + retained literal-skills pending Read"
        );
        assert!(reader.view(&ask(Page::Skills)).partial());
        saw_fallback = true;
        break;
    }
    assert!(
        saw_fallback,
        "the independent cap sweep must exercise fallback without whole refusal"
    );
}

#[test]
fn p1_terminal_size_refusal_is_visible_and_does_not_repeat_reads_after_restart() {
    let bytes = first();
    let (root, path) = fixture(bytes.as_bytes());
    let mut reader = Reader::new(
        root.0.clone(),
        Some(root.home()),
        Caps {
            conv_cap: 1,
            ..Caps::SLICE
        },
    );
    round(&mut reader);
    expect(&reader, 0, 0);
    let view = reader.view(&ask(Page::Tools));
    assert!(view.partial());
    assert!(
        view.gaps
            .iter()
            .any(|gap| gap.kind == GapKind::OverCap && gap.count > 0)
    );
    drop(reader);
    append(
        &path,
        f::call("new", "new", "Bash", "model-b", "{}").as_bytes(),
    );
    let mut reader = open(&root);
    assert_eq!(
        native_bytes(&round(&mut reader)),
        0,
        "terminal refusal retries only after reset, even if cap or source changes"
    );
    expect(&reader, 0, 0);
    drop(reader);
    assert!(!matches!(
        disk::reset(&root.0, Duration::ZERO).expect("reset refused state"),
        disk::Reset::Busy
    ));
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 2, 0);
}

#[test]
fn p1_damaged_and_oversized_conversation_documents_rebuild_the_whole_source() {
    for oversized in [false, true] {
        let mut bytes = large();
        let (root, path) = fixture(bytes.as_bytes());
        let mut reader = open(&root);
        round(&mut reader);
        expect(&reader, 1, 0);
        drop(reader);
        let documents = files(&root.0.join("stats/tools/conv"));
        assert_eq!(documents.len(), 1);
        let encoded = fs::read(&documents[0]).expect("valid durable document");
        let lf = encoded
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("head LF");
        let mut damaged = encoded[..=lf].to_vec();
        if oversized {
            let size = usize::try_from(crate::tool_stats::doc::CONV_CAP).expect("document cap");
            damaged.resize(size + 1, b' ');
        }
        // Keep the valid version head. Failure must be damage/size, not Format.
        Fixture::rewrite(&documents[0], &damaged);
        let added = f::call(
            "new-after-damage",
            "new-after-damage",
            "Bash",
            "model-b",
            "{}",
        );
        append(&path, added.as_bytes());
        bytes.push_str(&added);
        let mut reopened = open(&root);
        let units = round(&mut reopened);
        let read: u64 = units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Read)
            .map(|unit| unit.bytes)
            .sum();
        assert!(
            read >= u64::try_from(bytes.len()).expect("whole source size"),
            "damage/oversize rebuilds from zero"
        );
        cold_equal_after_damage(bytes.as_bytes(), &reopened, 2, 0);
        drop(reopened);
        let mut again = open(&root);
        assert_eq!(
            native_bytes(&round(&mut again)),
            0,
            "repaired document survives restart"
        );
        expect(&again, 2, 0);
    }
}

#[test]
fn p7_codex_restart_and_append_keep_late_context_and_return_attribution() {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "codex", f::C, Some("work"), "store-a"),
        &f::event("ordinary", "epoch fixture"),
    );
    prove_epoch(&root);
    let bytes = f::codex_call("old", Some("turn-a"), "read");
    let path = root.codex("store-a", f::C, bytes.as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 1, 0);
    assert_eq!(
        reader.view(&ask(Page::Tools)).detail[0].model,
        crate::stats::Cell::Unknown
    );
    drop(reader);
    let mut reopened = open(&root);
    assert_eq!(native_bytes(&round(&mut reopened)), 0);
    expect(&reopened, 1, 0);
    drop(reopened);
    let added = f::context("turn-a", "model-a", "high")
        + &f::codex_result("old")
        + &f::codex_call("new", Some("turn-a"), "write");
    append(&path, added.as_bytes());
    let mut reopened = open(&root);
    let units = round(&mut reopened);
    assert_eq!(
        units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Read)
            .map(|unit| unit.bytes)
            .sum::<u64>(),
        u64::try_from(added.len()).expect("append size")
    );
    assert!(native_bytes(&units) <= u64::try_from(added.len()).expect("append size") + (1 << 20));
    expect(&reopened, 2, 1);
    let view = reopened.view(&ask(Page::Tools));
    assert_eq!(view.detail.len(), 2);
    assert!(view.detail.iter().all(
        |row| row.model == crate::stats::Cell::Value("model-a".into())
            && row.effort == crate::stats::Cell::Value("high".into())
    ));
    let cold = super::scan(root.0.clone(), Some(root.home()), Caps::SLICE);
    for page in [Page::Tools, Page::Skills, Page::Mcp] {
        assert_eq!(reopened.view(&ask(page)), cold.view(&ask(page)));
    }
}

fn retire(root: &Fixture, id: &str) -> String {
    let summary = format!(
        "tool=claude profile=fixture-profile config_home={} config_home_base=",
        root.store("store-a").display()
    );
    format!(
        "{{\"ts\":\"2026-11-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"old-worker\",\"target_slot\":\"spawned.0\",\"ref\":\"{id}\",\"summary\":\"{summary}\"}}\n"
    )
}

#[test]
fn p3_assoc_restores_retired_memberships_and_resumes_its_journal_cursor() {
    let root = Fixture::new();
    let mut journal = retire(&root, f::C);
    let padding = f::event("ordinary", "association padding");
    while journal.len() < (3 << 20) + 8192 {
        journal.push_str(&padding);
    }
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::D, Some("work"), "store-a"),
        &journal,
    );
    prove_epoch(&root);
    root.claude("store-a", f::D, b"");
    root.claude("store-a", f::C, first().as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 1, 0);
    let selected = Ask {
        scope: Scope::This(Some(f::S.into())),
        ..ask(Page::Tools)
    };
    assert_eq!(counts(&reader.view(&selected)), (1, 0));
    let encoded = fs::read(root.0.join("stats/tools/assoc")).expect("durable association state");
    let assoc = crate::tool_stats::doc::parse_assoc(&encoded).expect("valid association document");
    assert!(
        assoc
            .sources
            .values()
            .any(|state| state.cursor == u64::try_from(journal.len()).expect("journal length"))
    );
    let old = assoc
        .memberships
        .get(&(crate::tool_stats::Adapter::Claude, f::C.into()))
        .expect("retired conversation persisted");
    assert!(
        old.memberships
            .iter()
            .any(|member| member.session.as_deref() == Some(f::S)
                && member.role == crate::tool_stats::Role::Retired)
    );
    drop(reader);
    let mut reopened = open(&root);
    let units = round(&mut reopened);
    assert_eq!(native_bytes(&units), 0);
    assert_eq!(
        units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Journal)
            .map(|unit| unit.bytes)
            .sum::<u64>(),
        0,
        "unchanged journal resumes without re-read"
    );
    assert_eq!(counts(&reopened.view(&selected)), (1, 0));
    drop(reopened);
    let next = "01a08046-1974-7352-ade3-81a786200797";
    let added = retire(&root, next);
    append(
        &root.0.join("sessions/alpha/events.jsonl"),
        added.as_bytes(),
    );
    let native = f::call("new-retired", "new-retired", "Write", "model-b", "{}");
    root.claude("store-a", next, native.as_bytes());
    let mut reopened = open(&root);
    let units = round(&mut reopened);
    let journal_bytes: u64 = units
        .iter()
        .filter(|unit| unit.kind == UnitKind::Journal)
        .map(|unit| unit.bytes)
        .sum();
    assert!(journal_bytes >= u64::try_from(added.len()).expect("new retire length"));
    assert!(
        journal_bytes <= u64::try_from(added.len()).expect("new retire length") + (1 << 20),
        "journal resume excludes the distant prefix"
    );
    assert_eq!(
        units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Read)
            .map(|unit| unit.bytes)
            .sum::<u64>(),
        u64::try_from(native.len()).expect("new native source length"),
        "old conversation is not re-read"
    );
    expect(&reopened, 2, 0);
    assert_eq!(counts(&reopened.view(&selected)), (2, 0));
    assert!(
        reopened
            .view(&selected)
            .detail
            .iter()
            .all(|row| row.client == crate::stats::Cell::Unknown)
    );
    let cold = super::scan(root.0.clone(), Some(root.home()), Caps::SLICE);
    assert_eq!(reopened.view(&selected), cold.view(&selected));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "keep the frozen canonical-prefix acceptance oracle in one test"
)]
fn p1_view_cap_keeps_a_canonical_prefix_and_preserves_omitted_conversation_counts() {
    use crate::tool_stats::Adapter;
    let expected = [
        (Adapter::Claude, f::C),
        (Adapter::Claude, f::D),
        (Adapter::Codex, f::C),
    ];
    let mut found = false;
    // Admission outcomes are asserted from three hand-written conversations.
    // The sweep chooses an input bound, never an expected encoded size.
    for view_cap in (128..=16_384).step_by(128) {
        let (root, _) = fixture(first().as_bytes());
        root.live(
            "beta",
            &root.meta("beta", f::T, "claude", f::D, Some("work"), "store-a"),
            "",
        );
        root.live(
            "gamma",
            &root.meta(
                "gamma",
                "33333333-3333-4333-8333-333333333333",
                "codex",
                f::C,
                Some("work"),
                "store-b",
            ),
            "",
        );
        root.claude(
            "store-a",
            f::D,
            f::call("second", "second", "Write", "model-b", "{}").as_bytes(),
        );
        root.codex(
            "store-b",
            f::C,
            f::codex_call("third", None, "bash").as_bytes(),
        );
        let mut reader = Reader::new(
            root.0.clone(),
            Some(root.home()),
            Caps {
                view_cap,
                ..Caps::SLICE
            },
        );
        round(&mut reader);
        let Ok(encoded) = fs::read(root.0.join("stats/tools/view")) else {
            continue;
        };
        let published = crate::tool_stats::doc::parse_view(&encoded).expect("valid published view");
        let kept = published.convs.len();
        if kept == 0 || kept == 3 {
            continue;
        }
        assert!(kept < 3);
        let keys: Vec<_> = published
            .convs
            .iter()
            .map(|conv| (conv.summary.adapter, conv.summary.id.as_str()))
            .collect();
        assert_eq!(
            keys,
            expected[..kept],
            "longest fitting prefix, independent of discovery order"
        );
        assert_eq!(
            published.over_cap,
            u64::try_from(3 - kept).expect("omitted count")
        );
        assert!(u64::try_from(encoded.len()).expect("view bytes") <= view_cap);
        // A separately encoded next-prefix candidate cannot fit. This uses
        // codec bytes, not the reader's admission function or charge formula.
        let assoc = crate::tool_stats::doc::parse_assoc(
            &fs::read(root.0.join("stats/tools/assoc")).expect("association document"),
        )
        .expect("valid associations");
        let next = expected[kept];
        let conv_path = root
            .0
            .join(crate::tool_stats::store::conv_name(next.0, next.1));
        let conv = crate::tool_stats::doc::parse_conv(
            &fs::read(conv_path).expect("omitted authoritative conv"),
        )
        .expect("valid omitted conv");
        let mut extended = published.clone();
        extended.convs.push(crate::tool_stats::incr::publish(
            &conv.summary,
            assoc.memberships.get(&(next.0, next.1.into())),
        ));
        extended.over_cap -= 1;
        assert!(
            u64::try_from(crate::tool_stats::doc::render_view(&extended).len())
                .expect("next prefix bytes")
                > view_cap,
            "no longer canonical prefix fits"
        );

        let view = reader.view(&ask(Page::Tools));
        assert_eq!(counts(&view), (u64::try_from(kept).expect("kept count"), 0));
        assert!(view.partial());
        assert!(
            view.gaps
                .iter()
                .any(|gap| gap.kind == GapKind::ViewOverCap && gap.count == published.over_cap)
        );
        assert_eq!(
            files(&root.0.join("stats/tools/conv")).len(),
            3,
            "view truncation never discards authoritative counts"
        );
        drop(reader);
        let mut roomy = open(&root);
        assert_eq!(
            native_bytes(&round(&mut roomy)),
            0,
            "larger view reuses all authoritative summaries"
        );
        expect(&roomy, 3, 0);
        assert!(
            !roomy
                .view(&ask(Page::Tools))
                .gaps
                .iter()
                .any(|gap| gap.kind == GapKind::ViewOverCap)
        );
        found = true;
        break;
    }
    assert!(
        found,
        "a partial nonempty canonical view must be exercisable"
    );
}

#[test]
fn p1_s5_owns_the_epoch_and_tools_publishes_after_the_real_first_build() {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        &f::event("ordinary", "epoch fixture"),
    );
    root.claude("store-a", f::C, first().as_bytes());
    let mut reader = open(&root);
    round(&mut reader);
    expect(&reader, 1, 0);
    assert!(
        !root.0.join("stats").exists(),
        "Tools cannot create an S5 payload or epoch"
    );
    prove_epoch(&root);
    let units = round(&mut reader);
    assert_eq!(
        native_bytes(&units),
        0,
        "the in-memory summary is retained until publication is possible"
    );
    assert!(
        units.iter().any(|unit| unit.store_bytes > 0),
        "publication reports actual store IO"
    );
    assert!(!files(&root.0.join("stats/tools/conv")).is_empty());
    drop(reader);
    let mut reopened = open(&root);
    let units = round(&mut reopened);
    assert_eq!(native_bytes(&units), 0);
    assert!(
        units.iter().any(|unit| unit.store_bytes > 0),
        "durable loads report actual store IO"
    );
    expect(&reopened, 1, 0);
}
