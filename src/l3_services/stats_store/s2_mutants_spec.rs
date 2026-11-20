//! Requirement-derived controls for the eight shard-0 survivor dispositions.

use super::{
    Dir, Duration, Feed, Files, Journal, Limits, Maintainer, Memory, MemoryJournals, Next, Op,
    Outcome, Prepared, Read, Rig, Root, S, Shown, T, birth, check, complete, disk, full, limits,
    published, recover, seed,
};
use crate::stats::SourceGap;
use crate::stats_store::doc;
use std::io;

#[test]
fn s2_m3_event_at_durable_cursor_counts_before_ordered_catchup() {
    let mut rig = Rig::new(vec![Feed::live("a", Some(S), birth(S, 1))], 4096, 4096);
    rig.catchup(0);
    let cursor = rig.feeds[0].cursor;
    let event = birth(S, 2);
    rig.feeds[0].bytes.extend_from_slice(&event);
    rig.sync(0);
    rig.event(0, cursor, &event[..event.len() - 1]);
    assert_eq!(
        rig.feeds[0].cursor, cursor,
        "event publication never moves the cursor"
    );
    assert_eq!(
        super::view(&mut rig.dir, None).all.total,
        2,
        "the certificate counts immediately at the unread boundary"
    );
    rig.catchup(0);
    assert_eq!(
        super::view(&mut rig.dir, None).all.total,
        2,
        "ordered replay never counts the certificate twice"
    );
}

#[test]
fn s2_m4_published_generation_matches_the_durable_root() {
    let root = Root::new();
    let first = birth(S, 1);
    root.live("a", S, &first);
    let mut maintainer = Maintainer::new(&root.0, limits(), Duration::ZERO);
    let mut journal = first;
    let mut previous = None;
    for n in 1..=2 {
        if n == 2 {
            journal.extend(birth(S, 2));
            root.live("a", S, &journal);
        }
        let step = maintainer.step();
        let Outcome::Published { r#gen, .. } = step.outcome else {
            panic!("an appended certificate publishes: {step:?}");
        };
        let Read::Bytes(bytes) = Files::at(&root.0).read("stats/view", disk::ROOT_VIEW_CAP) else {
            panic!("published root bytes");
        };
        let durable = doc::parse_root(std::str::from_utf8(&bytes).expect("root text"))
            .expect("published root document");
        assert_eq!(
            r#gen, durable.generation,
            "the step reports the generation actually installed"
        );
        if let Some(old) = previous {
            assert!(r#gen > old, "another publication advances generation");
        }
        previous = Some(r#gen);
    }
}

fn replaced(dir: &mut dyn Dir, name: &str, bytes: Vec<u8>) {
    dir.apply(&Op::Remove(name.to_owned()))
        .expect("remove fixture document");
    dir.apply(&Op::Write {
        name: name.to_owned(),
        bytes,
    })
    .expect("replace fixture document");
}

#[test]
fn s2_m8_session_view_rejects_each_wrong_identity_field_independently() {
    for wrong_epoch in [true, false] {
        let mut dir = Memory::new();
        let _ = seed(&mut dir);
        let name = format!("stats/session/{S}.view");
        let Read::Bytes(bytes) = dir.read(&name, disk::SESSION_VIEW_CAP) else {
            panic!("seeded session view");
        };
        let mut document = doc::parse_session(std::str::from_utf8(&bytes).expect("session text"))
            .expect("seeded session document");
        if wrong_epoch {
            document.epoch += 1;
        } else {
            document.key = Some(T.to_owned());
        }
        let changed = doc::render_session(&document);
        assert!(
            doc::parse_session(&changed).is_ok(),
            "fixture is structurally valid, with exactly one wrong identity field"
        );
        replaced(&mut dir, &name, changed.into_bytes());
        let snapshot = |dir: &Memory| {
            dir.names()
                .into_iter()
                .map(|name| {
                    let bytes = dir.file(&name).map(<[u8]>::to_vec);
                    (name, bytes)
                })
                .collect::<Vec<_>>()
        };
        let before = snapshot(&dir);
        let Shown::Refused(refused) = disk::shown(&mut dir, Some(S)) else {
            panic!("either wrong epoch or wrong group must refuse the scoped view");
        };
        let (path, _) = refused.damage().expect("identity mismatch is DAMAGE");
        assert_eq!(path, name, "damage names the offending session view");
        assert_eq!(
            snapshot(&dir),
            before,
            "a reader never repairs or rewrites damage"
        );
    }
}

#[derive(Debug)]
struct ReadLog {
    inner: Memory,
    names: Vec<String>,
}

impl Dir for ReadLog {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        self.names.push(name.to_owned());
        self.inner.read(name, cap)
    }

    fn list(&mut self, name: &str) -> io::Result<Vec<String>> {
        self.inner.list(name)
    }

    fn apply(&mut self, op: &Op) -> io::Result<()> {
        self.inner.apply(op)
    }
}

#[test]
fn s2_m2_consistent_delta_does_not_read_an_unrelated_session_view() {
    let mut dir = Memory::new();
    let (mut journals, mut feeds) = seed(&mut dir);
    feeds[0].bytes.extend(birth(S, 8));
    journals.set(feeds[0].id(), 1, feeds[0].ino, &feeds[0].bytes);
    let Next::Work(work) = Prepared::read(&mut dir, &mut journals, &feeds[0].spec, limits(), None)
        .expect("prepare touched group")
    else {
        panic!("new complete line");
    };
    let mut logged = ReadLog {
        inner: dir,
        names: Vec::new(),
    };
    let plan = published(
        disk::publish(&mut logged, &mut journals, &work).expect("consistent delta publication"),
    );
    // all_views reads unrelated session views in the re-sum fallback. If that
    // fallback becomes lazy, revisit this cost observable rather than dropping it.
    let unrelated = format!("stats/session/{T}.view");
    assert!(
        !logged.names.contains(&unrelated),
        "by-difference publication reads only touched views on a consistent root"
    );
    complete(&plan, &mut logged);
    feeds[0].cursor = work.span().1;
    check(&mut logged, &feeds, limits().line);
}

#[test]
fn s2_m5_untracked_live_journal_uses_the_documented_root_path() {
    let root = Root::new();
    let bytes = birth(S, 1);
    root.live("a", S, &bytes);
    let mut journals = disk::FileJournals::at(&root.0);
    let observed = journals
        .window("live/a", 0, 4096)
        .expect("documented untracked live source");
    assert_eq!(
        observed.bytes, bytes,
        "the root-relative fallback reads the requested live journal"
    );
}

#[derive(Debug)]
struct JournalLog {
    inner: MemoryJournals,
    bytes: usize,
}

impl Journal for JournalLog {
    fn window(&mut self, source: &str, from: u64, max: u64) -> Result<disk::Window, SourceGap> {
        let window = self.inner.window(source, from, max)?;
        self.bytes += window.bytes.len();
        Ok(window)
    }
}

#[test]
fn s2_m6_tail_truncated_to_held_end_does_not_reread_held_content() {
    let mut dir = Memory::new();
    recover(&mut dir);
    let mut feed = Feed::live("a", Some(S), birth(S, 1));
    let mut journals = MemoryJournals::new();
    journals.set(feed.id(), 1, feed.ino, &feed.bytes);
    full(&mut dir, &mut journals, &mut feed);
    let cursor = feed.cursor;
    let lim = Limits {
        chunk: 128,
        line: 4096,
    };
    feed.bytes.extend_from_slice(&[b'x'; 384]);
    journals.set(feed.id(), 1, feed.ino, &feed.bytes);
    let Next::Holding(held) = Prepared::read(&mut dir, &mut journals, &feed.spec, lim, None)
        .expect("hold a partial line")
    else {
        panic!("one chunk cannot reach the tail end");
    };
    assert_eq!(held.span(), (cursor, cursor + lim.chunk));
    feed.bytes
        .truncate(usize::try_from(cursor + lim.chunk).expect("small fixture end"));
    journals.set(feed.id(), 1, feed.ino, &feed.bytes);
    let mut logged = JournalLog {
        inner: journals,
        bytes: 0,
    };
    let (next, read) = Prepared::read_counted(&mut dir, &mut logged, &feed.spec, lim, Some(held))
        .expect("the held bytes are still present at EOF");
    assert!(
        matches!(next, Next::Nothing),
        "a live partial tail never becomes a final line"
    );
    assert_eq!(read, 0, "no new bytes exist after the held end");
    assert_eq!(
        logged.bytes, 0,
        "the still-valid held content is not read again"
    );
    check(&mut dir, &[feed], limits().line);
}

#[test]
fn s2_m7_chunked_source_rejoins_the_current_round_after_other_sources() {
    let root = Root::new();
    let first = birth(S, 1);
    let mut large = first.clone();
    large.extend(birth(S, 2));
    large.extend(birth(S, 3));
    root.live("a", S, &large);
    root.live("b", T, &birth(T, 4));
    let lim = Limits {
        chunk: first.len() as u64,
        line: 4096,
    };
    let mut maintainer = Maintainer::new(&root.0, lim, Duration::ZERO);
    for source in ["live/a", "live/b", "live/a", "live/a"] {
        let step = maintainer.step();
        assert_eq!(
            step.source.as_deref(),
            Some(source),
            "unfinished sources rejoin behind waiting peers in this round"
        );
        assert!(
            matches!(step.outcome, Outcome::Published { .. }),
            "the scheduled source publishes its next complete chunk: {step:?}"
        );
    }
    let feeds = [
        Feed {
            cursor: large.len() as u64,
            active: true,
            ..Feed::live("a", Some(S), large)
        },
        Feed {
            cursor: birth(T, 4).len() as u64,
            active: true,
            ..Feed::live("b", Some(T), birth(T, 4))
        },
    ];
    check(&mut Files::at(&root.0), &feeds, lim.line);
}

#[test]
fn s2_fuzz_repeated_completed_catchup_is_an_oracle_checked_noop() {
    let mut rig = Rig::new(super::cases::fixtures(), 4096, 4096);
    for i in 0..rig.feeds.len() {
        rig.catchup(i);
    }
    let before: Vec<_> = rig
        .dir
        .names()
        .into_iter()
        .map(|name| {
            let bytes = rig
                .dir
                .file(&name)
                .expect("persisted fixture file")
                .to_vec();
            (name, bytes)
        })
        .collect();
    let cursors: Vec<_> = rig.feeds.iter().map(|feed| feed.cursor).collect();
    for i in 0..rig.feeds.len() {
        rig.catchup(i);
        rig.check();
    }
    let after: Vec<_> = rig
        .dir
        .names()
        .into_iter()
        .map(|name| {
            let bytes = rig
                .dir
                .file(&name)
                .expect("persisted fixture file")
                .to_vec();
            (name, bytes)
        })
        .collect();
    assert_eq!(after, before, "completed catch-up changes no durable file");
    assert_eq!(
        rig.feeds.iter().map(|feed| feed.cursor).collect::<Vec<_>>(),
        cursors,
        "completed source cursors stay fixed"
    );
}

#[test]
fn s2_retired_alias_names_do_not_create_recovery_work() {
    // Lead ruling A: only canonical retired names belong to recovery. A foreign
    // alias must not strand a healthy store behind RecoveryOwed.
    for alias in ["stats.old.01", "stats.old.+1"] {
        let mut dir = Memory::new();
        let (_, feeds) = seed(&mut dir);
        let baseline = disk::repair(&mut dir).expect("repair without a foreign alias");
        dir.apply(&Op::MakeDir(alias.to_owned()))
            .expect("plant a foreign alias directory");
        let top = dir.list("").expect("top-level fixture names");
        assert!(top.iter().any(|name| name == alias), "alias fixture exists");
        let before: Vec<_> = dir
            .names()
            .into_iter()
            .map(|name| {
                let bytes = dir.file(&name).expect("persisted fixture file").to_vec();
                (name, bytes)
            })
            .collect();
        assert!(
            !disk::owes(&mut dir).expect("inspect recovery debt"),
            "foreign alias {alias} does not create recovery debt"
        );
        let recovery = disk::recovery(&mut dir).expect("healthy store recovery");
        assert!(
            recovery.ops.is_empty(),
            "foreign alias {alias} requires no recovery: {recovery:?}"
        );
        let repair = disk::repair(&mut dir).expect("healthy store repair");
        assert_eq!(
            repair, baseline,
            "foreign alias {alias} does not change the normal rebuild plan"
        );
        assert!(
            !repair.ops.iter().any(|op| match op {
                Op::Write { name, .. }
                | Op::Remove(name)
                | Op::MakeDir(name)
                | Op::RemoveTree(name) => name == alias,
                Op::Rename { from, to } => from == alias || to == alias,
            }),
            "repair never names the foreign alias {alias}"
        );
        assert_eq!(dir.list("").expect("unchanged top-level names"), top);
        let after: Vec<_> = dir
            .names()
            .into_iter()
            .map(|name| {
                let bytes = dir.file(&name).expect("persisted fixture file").to_vec();
                (name, bytes)
            })
            .collect();
        assert_eq!(
            after, before,
            "foreign alias {alias} changes no durable file"
        );
        // The fold oracle does not interpret retired names. Do not use the
        // maximum_proof crash oracle, which intentionally assumes canonical names.
        check(&mut dir, &feeds, limits().line);
    }
}
