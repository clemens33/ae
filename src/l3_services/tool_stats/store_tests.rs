//! Crash safety of the tools publication beyond the frozen whole-recovery
//! pin: a crash INSIDE a recovery, at every prefix and again, still reads as
//! the old or the new generation whole; an unreadable view is never written
//! over.
#![allow(clippy::expect_used, reason = "test assertions")]

use super::*;
use crate::stats_store::disk::Memory;
use crate::tool_stats::StreamRole;
use crate::tool_stats::incr::{self, FileState, Source, StreamKey};

const ID: &str = "c1";

fn key() -> StreamKey {
    StreamKey {
        role: StreamRole::Parent,
        copy: [0; 16],
    }
}

fn conv(cursor: u64) -> Conv {
    let mut conv = Conv::new(Adapter::Claude, ID);
    conv.resume.set_file(
        key(),
        FileState {
            dev: 1,
            ino: 2,
            len: cursor,
            mtime: (3, 0),
            ctime: (4, 0),
            cursor,
            skipping: false,
            blocks: vec![[5; 32]],
        },
    );
    conv.summary.files.insert(key(), Source::Present);
    conv
}

fn view(conv: &Conv, generation: u64) -> Published {
    Published {
        generation,
        convs: vec![incr::publish(&conv.summary, None)],
        over_cap: 0,
    }
}

/// The published pair, both readable.
fn pair(dir: &mut Memory) -> (Conv, Published) {
    let conv = match load_conv(dir, Adapter::Claude, ID) {
        Loaded::Ok(conv) => conv,
        other => panic!("readable conversation: {other:?}"),
    };
    let view = match load_view(dir) {
        Loaded::Ok(view) => view,
        other => panic!("readable view: {other:?}"),
    };
    (conv, view)
}

/// Every way a recovery of `state` can crash and resume, until a recovery
/// finds nothing to do: each end state passes `check`.
fn settle(state: &Memory, depth: usize, trail: &str, check: &dyn Fn(&mut Memory, &str)) {
    let mut probe = state.clone();
    let plan = recovery(&mut probe).expect("listable subtree");
    if plan.ops.is_empty() {
        check(&mut probe, trail);
        return;
    }
    assert!(depth < 6, "recovery settles: {trail}");
    for upto in 1..=plan.ops.len() {
        let mut crashed = state.clone();
        plan.apply(&mut crashed, upto)
            .expect("real recovery prefix");
        settle(&crashed, depth + 1, &format!("{trail} r{upto}"), check);
    }
}

#[test]
fn a_crash_inside_any_recovery_still_reads_one_whole_generation() {
    let (old, new) = (conv(100), conv(200));
    let (old_view, new_view) = (view(&old, 1), view(&new, 2));
    let mut base = Memory::new();
    base.plant("stats/epoch", b"1\n");
    publish(&mut base, &old, &old_view)
        .expect("first plan")
        .apply(&mut base, usize::MAX)
        .expect("old state");
    let plan = publish(&mut base, &new, &new_view).expect("next plan");
    let name = conv_name(Adapter::Claude, ID);
    let commit = plan
        .ops
        .iter()
        .position(|op| matches!(op, Op::Rename { to, .. } if *to == name))
        .expect("one commit point");
    for upto in 0..=plan.ops.len() {
        let mut crashed = base.clone();
        plan.apply(&mut crashed, upto)
            .expect("real publication prefix");
        let expected = if upto > commit {
            (&new, &new_view)
        } else {
            (&old, &old_view)
        };
        settle(&crashed, 0, &format!("p{upto}"), &|dir, trail| {
            let (conv, view) = pair(dir);
            assert_eq!(
                (&conv, &view),
                expected,
                "one whole generation after {trail}"
            );
            let left = disk::Dir::list(dir, TOOLS).expect("listable");
            assert!(
                !left
                    .iter()
                    .any(|one| ["conv.tmp", "view.tmp", "assoc.tmp", "intent"]
                        .contains(&one.as_str())),
                "nothing staged left after {trail}: {left:?}"
            );
        });
    }
}

/// Q2 = A: the coverage published beside the view at every crash prefix and
/// inside any recovery: the pair is one whole generation, and a coverage
/// whose generation equals the view's is that generation's own (the loader
/// shows coverage only then), never another's.
#[test]
fn a_crash_at_any_paired_coverage_prefix_never_mixes_generations() {
    let (old, new) = (conv(100), conv(200));
    let (old_view, new_view) = (view(&old, 1), view(&new, 2));
    let coverage = |generation: u64, hidden: u64| Coverage {
        generation,
        hidden,
        ..Coverage::default()
    };
    let (old_cov, new_cov) = (coverage(1, 3), coverage(2, 5));
    let mut base = Memory::new();
    base.plant("stats/epoch", b"1\n");
    publish_with(&mut base, &old, &old_view, &old_cov)
        .expect("first plan")
        .apply(&mut base, usize::MAX)
        .expect("old state");
    let plan = publish_with(&mut base, &new, &new_view, &new_cov).expect("next plan");
    let name = conv_name(Adapter::Claude, ID);
    let commit = plan
        .ops
        .iter()
        .position(|op| matches!(op, Op::Rename { to, .. } if *to == name))
        .expect("one commit point");
    assert!(
        plan.ops
            .iter()
            .any(|op| matches!(op, Op::Rename { to, .. } if to == COVERAGE)),
        "positive control: the coverage is renamed in the same plan"
    );
    for upto in 0..=plan.ops.len() {
        let mut crashed = base.clone();
        plan.apply(&mut crashed, upto)
            .expect("real publication prefix");
        let expected = if upto > commit {
            (&new, &new_view)
        } else {
            (&old, &old_view)
        };
        settle(&crashed, 0, &format!("p{upto}"), &|dir, trail| {
            let (conv, view) = pair(dir);
            assert_eq!(
                (&conv, &view),
                expected,
                "one whole generation after {trail}"
            );
            let held = match load_coverage(dir) {
                Loaded::Ok(held) => held,
                other => panic!("readable coverage after {trail}: {other:?}"),
            };
            let own = if held.generation == 2 {
                &new_cov
            } else {
                &old_cov
            };
            assert_eq!(&held, own, "a generation's own coverage after {trail}");
            if upto == plan.ops.len() {
                assert_eq!(held.generation, view.generation, "paired at the end");
            }
            let left = disk::Dir::list(dir, TOOLS).expect("listable");
            assert!(
                !left.iter().any(|one| {
                    std::path::Path::new(one)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
                        || one == "intent"
                }),
                "nothing staged left after {trail}: {left:?}"
            );
        });
    }
}

/// A payload whose view cannot be read: an IO error, not damage.
struct Unreadable(Memory);

impl disk::Dir for Unreadable {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        if name == VIEW {
            Read::Unreadable("injected".to_owned())
        } else {
            self.0.read(name, cap)
        }
    }

    fn apply(&mut self, op: &Op) -> std::io::Result<()> {
        self.0.apply(op)
    }

    fn list(&mut self, dir: &str) -> std::io::Result<Vec<String>> {
        self.0.list(dir)
    }
}

#[test]
fn an_unreadable_view_is_never_written_over_and_a_damaged_one_is_rebuilt() {
    let (old, new) = (conv(100), conv(200));
    let mut base = Memory::new();
    base.plant("stats/epoch", b"1\n");
    publish(&mut base, &old, &view(&old, 1))
        .expect("first plan")
        .apply(&mut base, usize::MAX)
        .expect("old state");
    let mut io = Unreadable(base.clone());
    assert!(
        matches!(
            publish(&mut io, &new, &view(&new, 2)),
            Err(StoreError::Io(_))
        ),
        "an IO error on the view stops the publication"
    );
    let mut damaged = base;
    damaged.plant(VIEW, b"not a view\n");
    let plan = publish(&mut damaged, &new, &view(&new, 2)).expect("damage is rebuilt");
    plan.apply(&mut damaged, usize::MAX).expect("rebuilt view");
    assert_eq!(pair(&mut damaged), (new.clone(), view(&new, 2)));
}

// --- #288 mutants slot 1 survivors (store.rs:100, 138, 257, 335) ----------

/// store.rs:100: a conversation document that names another conversation is
/// damage, whether the adapter or the id differs.
#[test]
fn a_conversation_document_of_another_identity_is_rebuilt() {
    for (adapter, id) in [(Adapter::Codex, ID), (Adapter::Claude, "c2")] {
        let mut dir = Memory::new();
        dir.plant(
            &conv_name(Adapter::Claude, ID),
            &doc::render_conv(&Conv::new(adapter, id)),
        );
        assert!(
            matches!(
                load_conv(&mut dir, Adapter::Claude, ID),
                Loaded::Rebuild(DocError::Damaged("identity"))
            ),
            "{adapter:?} {id} under the name of (Claude, {ID})"
        );
    }
    let mut own = Memory::new();
    own.plant(
        &conv_name(Adapter::Claude, ID),
        &doc::render_conv(&conv(100)),
    );
    assert!(
        matches!(load_conv(&mut own, Adapter::Claude, ID), Loaded::Ok(_)),
        "control: its own document loads"
    );
}

/// store.rs:138: a tools directory without `conv/` gets it made in the plan
/// that renames into it; one that has it never makes it again.
#[test]
fn a_missing_conversation_directory_is_made_and_a_present_one_is_not() {
    let make = |plan: &disk::Plan| {
        plan.ops
            .iter()
            .filter(|op| matches!(op, Op::MakeDir(name) if name == CONVS))
            .count()
    };
    let one = conv(100);
    let mut missing = Memory::new();
    missing.plant("stats/epoch", b"1\n");
    missing.plant(SOURCES, b"other tools file\n");
    let plan = publish(&mut missing, &one, &view(&one, 1)).expect("plan");
    assert_eq!(make(&plan), 1, "conv/ is made: {:?}", plan.ops);
    plan.apply(&mut missing, usize::MAX)
        .expect("the rename lands in the made directory");
    assert_eq!(pair(&mut missing), (one.clone(), view(&one, 1)));
    let next = conv(200);
    let plan = publish(&mut missing, &next, &view(&next, 2)).expect("next plan");
    assert_eq!(make(&plan), 0, "conv/ exists: {:?}", plan.ops);
}

/// store.rs:257: a coverage document of exactly its cap is written; one
/// byte more is refused.
#[test]
fn a_coverage_document_of_exactly_its_cap_is_published() {
    use std::collections::BTreeSet;
    let session = |len: usize| BTreeSet::from([Some("a".repeat(len))]);
    let mut coverage = Coverage {
        generation: 1,
        unsupported: (0..4_000)
            .map(|n| (format!("t{n:05}"), session(1_000)))
            .collect(),
        ..Coverage::default()
    };
    let size = |coverage: &Coverage| doc::render_coverage(coverage).len();
    let short = usize::try_from(COVERAGE_CAP).expect("small cap") - size(&coverage);
    let last = coverage.unsupported.len() - 1;
    coverage.unsupported[last].1 = session(1_000 + short);
    assert_eq!(
        u64::try_from(size(&coverage)).expect("small"),
        COVERAGE_CAP,
        "control: the document is exactly the cap"
    );
    let one = conv(100);
    let mut base = Memory::new();
    base.plant("stats/epoch", b"1\n");
    let mut exact = base.clone();
    publish_with(&mut exact, &one, &view(&one, 1), &coverage)
        .expect("exactly the cap is published")
        .apply(&mut exact, usize::MAX)
        .expect("applied");
    assert_eq!(
        exact.file(COVERAGE).map(<[u8]>::len),
        Some(usize::try_from(COVERAGE_CAP).expect("small cap"))
    );
    coverage.unsupported[last].1 = session(1_001 + short);
    assert!(
        matches!(
            publish_with(&mut base, &one, &view(&one, 1), &coverage),
            Err(StoreError::Io(why)) if why.kind() == std::io::ErrorKind::InvalidData
        ),
        "one byte over the cap is refused"
    );
}

/// store.rs:335: a view-only publication never replaces a view of the same
/// or a later generation.
#[test]
fn a_view_publication_not_newer_than_the_held_view_is_stale() {
    let one = conv(100);
    let mut dir = Memory::new();
    dir.plant("stats/epoch", b"1\n");
    publish(&mut dir, &one, &view(&one, 2))
        .expect("first plan")
        .apply(&mut dir, usize::MAX)
        .expect("held view");
    for generation in [1, 2] {
        assert!(
            matches!(
                publish_view(&mut dir, &view(&one, generation)),
                Err(StoreError::Stale)
            ),
            "generation {generation} is not newer than 2"
        );
    }
    publish_view(&mut dir, &view(&one, 3))
        .expect("control: a newer view publishes")
        .apply(&mut dir, usize::MAX)
        .expect("applied");
    assert_eq!(pair(&mut dir).1.generation, 3);
}
