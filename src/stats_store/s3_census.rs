//! Frozen census contract: epoch, generation, cache reuse, and nonempty fragments.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Census, Dir, Life, Memory, MemoryJournals, Op, Read};

fn add(dir: &mut Memory, journals: &mut MemoryJournals, name: &str, n: u32) -> cases::Feed {
    let mut feed = cases::Feed::live(name, Some(cases::S), cases::birth(cases::S, n));
    feed.ino = 100 + u64::from(n);
    journals.set(name, 1, feed.ino, &feed.bytes);
    let work = cases::work(dir, journals, &feed.spec, &Life::default());
    let publication = cases::plan(disk::publish(dir, journals, &work).expect("add census source"));
    cases::complete(dir, &publication);
    feed.active = true;
    feed.cursor = u64::try_from(feed.bytes.len()).expect("fixture length");
    feed
}

#[test]
fn s3_census_reloads_changed_generation_after_publication_and_removal() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let old = add(&mut dir, &mut journals, "live/old", 1);
    cases::check(&mut dir, std::slice::from_ref(&old));
    let first = disk::census(&mut dir, &Census::default()).expect("first census");
    assert_eq!(first.sources.len(), 1);
    assert_eq!(first.sources["live/old"].stamp, (1, old.cursor));

    let mut new = add(&mut dir, &mut journals, "live/new", 2);
    cases::check(&mut dir, &[old.clone(), new.clone()]);
    let second = disk::census(&mut dir, &first).expect("changed generation census");
    assert_eq!(second.sources.len(), 2, "new source appears this round");
    let entry = second.sources.get("live/new").expect("new source present");
    assert_eq!(entry.key.as_deref(), Some(cases::S));
    assert!(entry.counted);
    assert_eq!(entry.stamp, (2, new.cursor), "second publication stamp");

    let removal = cases::plan(
        disk::remove(&mut dir, Some(cases::S), "live/new", None).expect("remove new source"),
    );
    cases::complete(&mut dir, &removal);
    new.active = false;
    cases::check(&mut dir, &[old, new]);
    let third = disk::census(&mut dir, &second).expect("removal census");
    assert_eq!(
        third.sources, first.sources,
        "removed source disappears this round"
    );
}

#[test]
fn s3_census_does_not_reuse_a_matching_generation_from_a_dead_epoch() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let old = add(&mut dir, &mut journals, "live/old", 1);
    cases::check(&mut dir, std::slice::from_ref(&old));
    let first = disk::census(&mut dir, &Census::default()).expect("old epoch census");
    assert_eq!(first.sources["live/old"].stamp.0, 1);

    // Another writer may repair the store while the maintainer retains first.
    // Its next epoch starts generations again at 1 in the same group S.
    let reset = disk::repair(&mut dir).expect("new epoch");
    cases::complete(&mut dir, &reset);
    let new = add(&mut dir, &mut journals, "live/new", 2);
    cases::check(&mut dir, std::slice::from_ref(&new));
    let second = disk::census(&mut dir, &first).expect("new epoch census");
    assert_eq!(second.sources.len(), 1);
    assert!(
        !second.sources.contains_key("live/old"),
        "dead epoch source discarded"
    );
    let entry = second
        .sources
        .get("live/new")
        .expect("new epoch source present");
    assert_eq!(entry.key.as_deref(), Some(cases::S));
    assert_eq!(
        entry.stamp,
        (1, new.cursor),
        "coincident generation in new epoch"
    );
}

struct Counted<'a> {
    inner: &'a mut Memory,
    views: Vec<String>,
}

impl Dir for Counted<'_> {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        if name.starts_with("stats/session/") && name.ends_with(".view") {
            self.views.push(name.to_owned());
        }
        self.inner.read(name, cap)
    }

    fn apply(&mut self, op: &Op) -> std::io::Result<()> {
        self.inner.apply(op)
    }

    fn list(&mut self, dir: &str) -> std::io::Result<Vec<String>> {
        self.inner.list(dir)
    }
}

#[test]
fn s3_census_reads_only_views_whose_group_generation_changed() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = cases::seed(&mut dir, &mut journals);
    let first = disk::census(&mut dir, &Census::default()).expect("initial census");
    let mut counted = Counted {
        inner: &mut dir,
        views: Vec::new(),
    };
    let second = disk::census(&mut counted, &first).expect("unchanged census");
    assert_eq!(second.sources, first.sources);
    assert!(
        counted.views.is_empty(),
        "unchanged groups do not reread their views: {:?}",
        counted.views
    );

    feeds.push(add(&mut dir, &mut journals, "live/new", 4));
    cases::check(&mut dir, &feeds);
    let mut counted = Counted {
        inner: &mut dir,
        views: Vec::new(),
    };
    let third = disk::census(&mut counted, &second).expect("one changed group census");
    assert!(third.sources.contains_key("live/new"));
    assert_eq!(
        counted.views,
        vec![format!("stats/session/{}.view", cases::S)],
        "only changed S is read; T and idless groups reuse their entries"
    );
}

#[test]
fn s3_census_never_schedules_a_fragment_only_group_as_empty() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = cases::seed(&mut dir, &mut journals);
    let removal = cases::plan(
        disk::remove(&mut dir, Some(cases::T), "live/b", None).expect("remove T's own source"),
    );
    cases::complete(&mut dir, &removal);
    feeds[1].active = false;
    cases::check(&mut dir, &feeds);
    // T still holds the explicit birth(T, 2) from S's journal, with no own row.
    let census = disk::census(&mut dir, &Census::default()).expect("fragment-only census");
    assert!(
        !census.empty.contains(&Some(cases::T.to_owned())),
        "foreign counts prevent pruning T"
    );
    assert!(
        !census.empty.contains(&Some(cases::S.to_owned())),
        "own source prevents pruning S"
    );
}

#[test]
fn s3_census_schedules_an_empty_unindexed_removal_leftover() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feed = add(&mut dir, &mut journals, "live/only", 1);
    cases::check(&mut dir, std::slice::from_ref(&feed));
    let removal = cases::plan(
        disk::remove(&mut dir, Some(cases::S), "live/only", None).expect("remove only source"),
    );
    // Crash after the committed intent is cleared, before leftover cleanup.
    let view_path = format!("stats/session/{}.view", cases::S);
    let before_cleanup = removal
        .ops
        .iter()
        .position(|op| matches!(op, Op::Remove(path) if path == &view_path))
        .expect("postcommit empty-view cleanup");
    removal
        .apply(&mut dir, before_cleanup)
        .expect("crash before leftover cleanup");
    assert!(
        disk::recovery(&mut dir)
            .expect("intent already cleared")
            .ops
            .is_empty()
    );
    feed.active = false;
    cases::check(&mut dir, &[feed]);
    let Read::Bytes(root) = dir.read("stats/view", disk::ROOT_VIEW_CAP) else {
        panic!("materialized root after removal");
    };
    assert!(
        !String::from_utf8(root)
            .expect("root UTF-8")
            .contains(cases::S),
        "root no longer indexes S"
    );
    assert!(
        matches!(
            dir.read(
                &format!("stats/session/{}.view", cases::S),
                disk::SESSION_VIEW_CAP
            ),
            Read::Bytes(_)
        ),
        "empty view survives removal until prune"
    );
    let census = disk::census(&mut dir, &Census::default()).expect("empty leftover census");
    assert!(census.sources.is_empty());
    assert_eq!(
        census.empty,
        vec![Some(cases::S.to_owned())],
        "empty unindexed view is scheduled for pruning"
    );
}
