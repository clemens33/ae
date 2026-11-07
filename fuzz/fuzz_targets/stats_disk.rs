#![no_main]
#![allow(
    clippy::expect_used,
    reason = "fuzz assertions over valid generated contract operations"
)]

use ae::stats_store::disk::{self, Decision, Dir, Memory, Next, Op, Prepared, Read, Shown, Why};
use ae::stats_store::{Doc, parse_count};
use libfuzzer_sys::fuzz_target;

#[path = "../../src/stats_store/s2_cases.rs"]
mod cases;

#[path = "../../src/stats_store/s3_cases.rs"]
mod s3_cases;
#[path = "../../src/stats_store/s3_keys.rs"]
mod s3_keys;

#[path = "../../src/stats_store/s4_cases.rs"]
mod s4_cases;

use cases::{Rig, S, T, birth, complete, published, recover};

// The cap decision at the real Files door is tested by the frozen unit tests. This
// external-read stand-in fuzzes how the shared decisions handle that door's refusal,
// without allocating 64 MiB on every fuzz iteration.
struct OverCap<'a> {
    dir: &'a mut Memory,
    name: &'a str,
}
impl Dir for OverCap<'_> {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        if name == self.name {
            Read::Oversized
        } else {
            self.dir.read(name, cap)
        }
    }
    fn apply(&mut self, op: &Op) -> std::io::Result<()> {
        self.dir.apply(op)
    }
    fn list(&mut self, dir: &str) -> std::io::Result<Vec<String>> {
        self.dir.list(dir)
    }
}

fn committed(plan: &disk::Plan, cut: usize, group: &str) -> bool {
    plan.ops.iter().take(cut).any(|op| matches!(op, Op::Rename { to, .. } if to == &format!("stats/session/{group}.evidence")))
}

fn exercise(data: &[u8]) {
    let kind = data.first().copied().unwrap_or_default() % 6;
    let index = usize::from(data.get(1).copied().unwrap_or_default());
    let mut rig = Rig::new(cases::fixtures(), 4096, 4096);
    for i in 0..rig.feeds.len() {
        rig.catchup(i);
    }
    match kind {
        0 | 1 => {
            rig.feeds[0].bytes.extend(birth(T, 77));
            rig.feeds[0].bytes.extend(birth(S, 78));
            rig.sync(0);
            let work = rig.ready(0);
            let plan = published(
                disk::publish(&mut rig.dir, &mut rig.journals, &work).expect("valid publish"),
            );
            let cut = if kind == 1 {
                plan.ops.iter().position(|o|matches!(o,Op::Rename{to,..} if to==&format!("stats/session/{S}.evidence"))).expect("primary evidence rename")+1
            } else {
                index % (plan.ops.len() + 1)
            };
            plan.apply(&mut rig.dir, cut).expect("publish prefix");
            let changed = committed(&plan, cut, S);
            if kind == 1 {
                let redo = disk::recovery(&mut rig.dir).expect("recover interrupted publish");
                let again =
                    usize::from(data.get(2).copied().unwrap_or_default()) % (redo.ops.len() + 1);
                redo.apply(&mut rig.dir, again).expect("recovery prefix");
            }
            recover(&mut rig.dir);
            if changed {
                rig.accept(0, &work);
            }
            rig.check();
            rig.catchup(0);
            rig.check();
        }
        2 => {
            let plan = disk::repair(&mut rig.dir).expect("valid repair");
            let cut = index % (plan.ops.len() + 1);
            let changed = plan
                .ops
                .iter()
                .take(cut)
                .any(|op| matches!(op,Op::Rename{from,..} if from=="stats"));
            plan.apply(&mut rig.dir, cut).expect("repair prefix");
            recover(&mut rig.dir);
            if changed {
                for f in &mut rig.feeds {
                    f.active = false;
                    f.cursor = 0;
                    f.early.clear();
                }
                if let Shown::Ready { view, .. } = disk::shown(&mut rig.dir, None) {
                    assert_eq!(view.all.total, 0);
                }
            }
            rig.check();
            for i in 0..rig.feeds.len() {
                rig.catchup(i);
            }
        }
        3 => {
            let at = rig.feeds[0].bytes.len() as u64;
            let line = birth(T, 99);
            rig.feeds[0].bytes.extend(&line);
            rig.sync(0);
            let old = rig.ready(0);
            rig.event(0, at, &line[..line.len() - 1]);
            // Same offset is a set insertion, never a second birth.
            let f = &rig.feeds[0];
            let plan = published(
                disk::event(&mut rig.dir, f.group(), f.id(), at, &line[..line.len() - 1])
                    .expect("repeated event"),
            );
            complete(&plan, &mut rig.dir);
            rig.apply(0, &old);
            rig.check();
        }
        4 => {
            rig.feeds[0].bytes.extend(birth(S, 88));
            rig.sync(0);
            let old = rig.ready(0);
            rig.feeds[0].ino += 1;
            rig.feeds[0].bytes = birth(S, 90);
            rig.sync(0);
            assert!(matches!(
                disk::publish(&mut rig.dir, &mut rig.journals, &old).expect("fresh witness"),
                Decision::Discard(Why::Witness)
            ));
            let reset = rig.ready(0);
            assert!(reset.is_reset());
            rig.apply(0, &reset);
        }
        _ => {
            rig.feeds[0].bytes.extend(birth(S, 88));
            rig.sync(0);
            let a = rig.ready(0);
            let b = rig.ready(0);
            rig.apply(0, &a);
            assert!(matches!(
                disk::publish(&mut rig.dir, &mut rig.journals, &b).expect("stale stamp"),
                Decision::Discard(Why::Stamp)
            ));
            rig.check();
        }
    }

    rig.feeds[0].bytes.extend(birth(S, 123));
    rig.sync(0);
    let cap_work = rig.ready(0);
    let capped = [
        ("stats/epoch".to_owned(), Doc::Epoch),
        ("stats/intent".to_owned(), Doc::Intent),
        ("stats/view".to_owned(), Doc::RootView),
        (format!("stats/session/{S}.view"), Doc::SessionView),
        (format!("stats/session/{S}.evidence"), Doc::Evidence),
    ];
    let (name, doc) = &capped[index % capped.len()];
    let mut over = OverCap {
        dir: &mut rig.dir,
        name,
    };
    let before = parse_count(*doc);
    let _ = disk::shown(&mut over, Some(S));
    let _ = disk::resum(&mut over);
    let _ = disk::recovery(&mut over);
    let _ = disk::publish(&mut over, &mut rig.journals, &cap_work);
    assert_eq!(
        parse_count(*doc),
        before,
        "over-cap file reached its parser"
    );

    // Hostile documents go through the real shared decision/parsing functions. Every payload
    // kind is reached independently; epoch and intent inputs naturally exercise their byte caps.
    let names = [
        "stats/epoch".to_owned(),
        "stats/intent".to_owned(),
        "stats/view".to_owned(),
        format!("stats/session/{S}.view"),
        format!("stats/session/{S}.evidence"),
    ];
    let hostile = &data[..data.len().min(16384)];
    for name in names {
        let mut broken = Memory::new();
        // Start with valid other files, so an earlier missing document cannot hide the parser.
        for existing in rig.dir.names() {
            if let Some(bytes) = rig.dir.file(&existing) {
                broken.plant(&existing, bytes);
            }
        }
        broken.plant(&name, hostile);
        let _ = disk::shown(&mut broken, Some(S));
        let _ = disk::resum(&mut broken);
        let _ = disk::recovery(&mut broken);
        let _ = disk::repair(&mut broken);
        let mut journals = disk::MemoryJournals::new();
        journals.set(rig.feeds[0].id(), 1, rig.feeds[0].ino, &rig.feeds[0].bytes);
        if let Ok(Next::Work(work)) = Prepared::read(
            &mut broken,
            &mut journals,
            &rig.feeds[0].spec,
            rig.limits,
            None,
        ) {
            let _ = disk::publish(&mut broken, &mut journals, &work);
        }
    }
}

fuzz_target!(|data: &[u8]| {
    exercise(data);
    s3_cases::fuzz_case(data);
    s3_keys::exercise(data);
    s4_cases::exercise(data);
});
