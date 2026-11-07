//! Frozen additive-key parser shapes. Generated valid surrounding records keep the changed parser reachable.
#![allow(clippy::expect_used, reason = "contract fixture assertions")]
use super::s3_cases as cases;
#[cfg(test)]
use crate as ae;
use ae::stats_store::disk::{self, Dir, Life, Memory, MemoryJournals, Read, Shown, SourceSpec};

// Only generated fixtures reach this editor. It edits a known top-level scalar/array field,
// preserving all other bytes and the document's record count.
pub fn field(bytes: &[u8], prefix: &str, key: &str, value: Option<&str>) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).expect("generated UTF-8");
    let mut changed = false;
    let mut out = String::new();
    for original in text.lines() {
        let mut line = original.to_owned();
        if line.starts_with(prefix) {
            changed = true;
            let needle = format!(",\"{key}\":");
            if let Some(start) = line.find(&needle) {
                let at = start + needle.len();
                let tail = &line[at..];
                let length = if tail.starts_with('[') {
                    tail.find(']').expect("generated array end") + 1
                } else if let Some(rest) = tail.strip_prefix('"') {
                    rest.find('"').expect("generated string end") + 2
                } else {
                    tail.find([',', '}']).expect("generated scalar end")
                };
                line.replace_range(start..at + length, "");
            }
            if let Some(value) = value {
                let end = line.len() - 1;
                line.insert_str(end, &format!(",\"{key}\":{value}"));
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    assert!(changed, "fixture record prefix {prefix}");
    out.into_bytes()
}

fn bytes(dir: &mut Memory, name: &str) -> Vec<u8> {
    match dir.read(name, disk::EVIDENCE_CAP) {
        Read::Bytes(bytes) => bytes,
        other => panic!("fixture {name}: {other:?}"),
    }
}
fn edit(dir: &mut Memory, name: &str, prefix: &str, key: &str, value: Option<&str>) {
    let edited = field(&bytes(dir, name), prefix, key, value);
    dir.plant(name, &edited);
}

#[allow(
    clippy::too_many_lines,
    reason = "explicit additive-key contract matrix"
)]
pub fn exercise(data: &[u8]) {
    let mode = data.first().copied().unwrap_or_default() % 12;
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = cases::seed(&mut dir, &mut journals);
    let session = format!("stats/session/{}.view", cases::S);
    let evidence = format!("stats/session/{}.evidence", cases::S);
    let root_row = format!("{{\"group\":\"{}\"", cases::S);
    let mut bad_root = false;
    let mut bad_view = false;
    let mut bad_evidence = false;
    match mode {
        0 => edit(&mut dir, "stats/view", "{\"group\":", "ids", None),
        1 => {
            edit(
                &mut dir,
                "stats/view",
                &format!("{{\"group\":\"{}\"", cases::T),
                "ids",
                Some("[\"live/a\"]"),
            );
            bad_root = true;
        }
        2 => {
            edit(&mut dir, "stats/view", &root_row, "ids", Some("[]"));
            bad_root = true;
        }
        3 => {
            let mut feed = cases::Feed::live("live/c", Some(cases::S), cases::birth(cases::S, 9));
            journals.set(feed.id(), 1, 80, &feed.bytes);
            let work = cases::work(&mut dir, &mut journals, &feed.spec, &Life::default());
            let publication =
                cases::plan(disk::publish(&mut dir, &mut journals, &work).expect("second source"));
            cases::complete(&mut dir, &publication);
            feed.active = true;
            feed.cursor = feed.bytes.len() as u64;
            feeds.push(feed);
            edit(
                &mut dir,
                "stats/view",
                &root_row,
                "ids",
                Some("[\"live/c\",\"live/a\"]"),
            );
            bad_root = true;
        }
        4 | 5 => {
            let hash = format!("\"{}\"", "a".repeat(64));
            let value = (mode == 4).then_some(hash.as_str());
            edit(&mut dir, &session, "{\"row\":", "org", value);
            edit(&mut dir, &session, "{\"row\":", "fp", value);
            edit(&mut dir, &evidence, "{\"source\":", "fp", value);
        }
        6 | 10 => {
            let failed = SourceSpec::Failed {
                id: "live/a".to_owned(),
                label: "live/a".to_owned(),
                session: Some(cases::S.to_owned()),
                gap: ae::stats::SourceGap::Missing,
            };
            let work = cases::work(&mut dir, &mut journals, &failed, &Life::default());
            let publication =
                cases::plan(disk::publish(&mut dir, &mut journals, &work).expect("failed row"));
            cases::complete(&mut dir, &publication);
            feeds[0].spec = failed;
            feeds[0].cursor = 0;
            edit(
                &mut dir,
                &session,
                "{\"row\":",
                "gap",
                if mode == 6 {
                    None
                } else {
                    Some("\"unknown-gap\"")
                },
            );
            bad_view = mode == 10;
        }
        7 | 8 => {
            edit(
                &mut dir,
                &session,
                "{\"row\":",
                if mode == 7 { "org" } else { "fp" },
                Some("\"not-a-hash\""),
            );
            bad_view = true;
        }
        9 => {
            edit(
                &mut dir,
                &evidence,
                "{\"source\":",
                "fp",
                Some("\"not-a-hash\""),
            );
            bad_evidence = true;
        }
        _ => {
            edit(&mut dir, &session, "{\"row\":", "gap", Some("\"missing\""));
            bad_view = true;
        }
    }
    if bad_root {
        assert!(
            matches!(disk::shown(&mut dir, None), Shown::Refused(_)),
            "hostile root mode {mode}"
        );
    } else if bad_view {
        assert!(
            matches!(disk::shown(&mut dir, Some(cases::S)), Shown::Refused(_)),
            "hostile view mode {mode}"
        );
    } else if !bad_evidence {
        cases::check(&mut dir, &feeds);
    }
    let removal = disk::remove(&mut dir, Some(cases::S), "live/a", None);
    if bad_evidence {
        assert!(
            matches!(removal, Err(disk::Refused::Damaged { .. })),
            "hostile provenance must reach evidence parser: {removal:?}"
        );
    }
    let _ = disk::resum(&mut dir);
    let _ = disk::recovery(&mut dir);
    // Exercise arbitrary suffixes through the same source-row parser while other files stay valid.
    if let Some(suffix) = data.get(3..) {
        let mut hostile = bytes(&mut dir, &session);
        hostile.extend_from_slice(&suffix[..suffix.len().min(1024)]);
        dir.plant(&session, &hostile);
        let _ = disk::shown(&mut dir, Some(cases::S));
    }
}
