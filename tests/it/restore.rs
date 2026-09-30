//! Driver pins for the restore predicate, the beat and the ledger trim. The
//! black-box contract is `restore_spec.rs`, written by another agent.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what PRODUCT code may reach"
)]

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, SystemTime};

use ae::meta::{Selector, ServerSelector};
use ae::restore::{Decision, Fact, Ledger, WINDOW_SECS, judge, ledger, pins_clean_stop};
use ae::session::SessionRead;
use ae::tmux::Evidence;
use ae::watchdog_glue::{beat_modified, beat_path, touch_beat};

use super::cli::OwnedScratch;

fn fact(name: &str, on: &str, launched: i64, ledger: Ledger, beat: Option<i64>) -> Fact {
    Fact {
        name: name.to_owned(),
        server: ServerSelector::Positive(Selector::Name(on.to_owned())),
        live: false,
        launched: Evidence::At(launched),
        ledger,
        beat: beat.map_or(Evidence::Silent, Evidence::At),
    }
}

#[test]
fn the_crashed_cohort_restores_and_every_other_class_is_named_and_skipped() {
    use Decision::{Abandoned, Damaged, Live, NoBeat, NoServer, Restore, Stopped, Unlaunched};
    use Ledger::Clear;
    let newest = 90_000;
    let mut live = fact("live", "s", 50_000, Clear, Some(newest + 500));
    live.live = true;
    let mut unlaunched = fact("unlaunched", "s", 0, Clear, Some(newest));
    unlaunched.launched = Evidence::Silent;
    let mut nowhere = fact("nowhere", "s", 1000, Clear, Some(newest));
    nowhere.server = ServerSelector::Missing;
    let facts = [
        fact("cohort-max", "s", 1000, Clear, Some(newest)),
        fact("edge", "s", 1000, Clear, Some(newest - WINDOW_SECS)),
        fact(
            "abandoned",
            "s",
            1000,
            Clear,
            Some(newest - WINDOW_SECS - 1),
        ),
        fact("stopped", "s", 1000, Ledger::Stopped, Some(newest + 400)),
        fact("damaged", "s", 1000, Ledger::Damaged, Some(newest + 300)),
        fact("unwatched", "s", 1000, Clear, None),
        // A beat from before the current launch belongs to an earlier run.
        fact("prior-run", "s", newest + 100, Clear, Some(newest)),
        live,
        unlaunched,
        nowhere,
        fact("other-server", "t", 1000, Clear, Some(1500)),
    ];
    // The live, stopped and damaged beats are all newer than `newest` and none
    // of them may move the window: only unstopped, unlive, readable beats do.
    assert_eq!(
        judge(&facts),
        [
            Restore, Restore, Abandoned, Stopped, Damaged, NoBeat, NoBeat, Live, Unlaunched,
            NoServer, Restore
        ]
    );
}

type Line = (&'static str, i64, &'static str);

fn scratch_with(tag: &str, lines: &[Line], junk: bool) -> (OwnedScratch, Ledger) {
    let scratch = OwnedScratch::root("rt", tag);
    let mut body = String::new();
    for (action, at, summary) in lines {
        let ts = ae::time::Timestamp::from_epoch(*at);
        let _ = writeln!(
            body,
            "{{\"ts\":\"{ts}\",\"actor\":\"human\",\"action\":\"{action}\",\"summary\":\"{summary}\"}}"
        );
    }
    if junk {
        body.push_str("{not json\n");
    }
    let wrote = std::fs::write(scratch.join("events.jsonl"), body);
    wrote.unwrap_or_else(|why| panic!("fixture ledger: {why}"));
    let read = SessionRead::open(&scratch).ok();
    let verdict = ledger(read.as_ref(), 1000);
    (scratch, verdict)
}

#[test]
fn the_ledger_fails_closed_and_counts_only_stops_since_the_launch() {
    const CLEAN: &str = "stopped: verified gone on its recorded server";
    let cases: [(&[Line], bool, Ledger); 10] = [
        (&[("stop-result", 999, CLEAN)], false, Ledger::Clear),
        (&[("stop-result", 1000, CLEAN)], false, Ledger::Stopped),
        (
            &[("stop-result", 1001, "already stopped")],
            false,
            Ledger::Stopped,
        ),
        (
            &[("stop-result", 1001, "FAILED: unavailable")],
            false,
            Ledger::Clear,
        ),
        (
            &[("stop-request", 1001, "stop requested")],
            false,
            Ledger::Stopped,
        ),
        (
            &[("stop-request", 900, "stop requested")],
            false,
            Ledger::Clear,
        ),
        (
            &[
                ("stop-request", 1001, "x"),
                ("stop-result", 1002, "FAILED: a"),
            ],
            false,
            Ledger::Clear,
        ),
        (
            &[
                ("stop-request", 1001, "x"),
                ("stop-result", 1002, "FAILED: a"),
                ("stop-request", 1003, "x"),
            ],
            false,
            Ledger::Stopped,
        ),
        // A memo that QUOTES a clean stop is not one.
        (&[("memo", 1500, CLEAN)], false, Ledger::Clear),
        // One unreadable line may have been the stop: unknown, not clear.
        (&[("stop-result", 999, CLEAN)], true, Ledger::Damaged),
    ];
    for (index, (lines, junk, want)) in cases.into_iter().enumerate() {
        let (_scratch, got) = scratch_with(&format!("l{index}"), lines, junk);
        assert_eq!(got, want, "case {index}: {lines:?} junk={junk}");
    }
    assert_eq!(ledger(None, 1000), Ledger::Damaged, "an unreadable ledger");
}

fn mtime(path: &Path) -> SystemTime {
    std::fs::symlink_metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or_else(|why| panic!("mtime: {why}"))
}

#[test]
fn the_beat_is_an_empty_file_refreshed_by_exclusive_temp_and_never_written_through_a_link() {
    let scratch = OwnedScratch::root("rt", "beat");
    let dir: &Path = &scratch;
    assert_eq!(beat_modified(dir), Evidence::Silent);
    touch_beat(dir).expect("first beat");
    let beat = beat_path(dir);
    assert_eq!(std::fs::metadata(&beat).expect("beat").len(), 0);
    let old = SystemTime::now() - Duration::from_hours(2);
    std::fs::File::options()
        .write(true)
        .open(&beat)
        .and_then(|file| file.set_modified(old))
        .expect("aged beat");
    touch_beat(dir).expect("refresh");
    assert!(mtime(&beat) > old + Duration::from_hours(1));
    assert!(matches!(beat_modified(dir), Evidence::At(_)));
    let leftovers = std::fs::read_dir(dir)
        .expect("dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
        .count();
    assert_eq!(leftovers, 0, "a staged temp was left behind");

    // A link planted AT the beat is replaced, never followed.
    let target = scratch.join("target");
    std::fs::write(&target, "keep").expect("target");
    std::fs::remove_file(&beat).expect("rm beat");
    std::os::unix::fs::symlink(&target, &beat).expect("planted link");
    assert_eq!(beat_modified(dir), Evidence::Unreadable, "a link is damage");
    let before = mtime(&target);
    touch_beat(dir).expect("replace the link");
    assert!(std::fs::symlink_metadata(&beat).expect("beat").is_file());
    assert_eq!(std::fs::read_to_string(&target).expect("target"), "keep");
    assert_eq!(mtime(&target), before, "the link's target was touched");

    // A link planted at the predictable TEMP name is refused, never truncated.
    let staged = dir.join(format!(".watchdog-beat.tmp.{}", std::process::id()));
    std::os::unix::fs::symlink(&target, &staged).expect("planted temp");
    assert!(touch_beat(dir).is_err(), "a taken temp name must refuse");
    assert_eq!(std::fs::read_to_string(&target).expect("target"), "keep");

    // A directory where the beat belongs is damage.
    std::fs::remove_file(&staged).expect("rm temp");
    std::fs::remove_file(&beat).expect("rm beat");
    std::fs::create_dir(&beat).expect("dir beat");
    assert_eq!(beat_modified(dir), Evidence::Unreadable);
}

#[test]
fn resume_retention_keeps_only_the_newest_clean_stop_when_the_window_dropped_it() {
    let scratch = OwnedScratch::root("rt", "trim");
    let store = ae::store::open(&scratch);
    let clean = |n: u32| {
        format!(
            "{{\"ts\":\"2026-09-30T10:00:0{n}Z\",\"actor\":\"human\",\"action\":\"stop-result\",\"summary\":\"already stopped\"}}\n"
        )
    };
    let failed = "{\"ts\":\"2026-09-30T10:01:00Z\",\"actor\":\"human\",\"action\":\"stop-result\",\"summary\":\"FAILED: no\"}\n";
    let other = "{\"ts\":\"2026-09-30T10:02:00Z\",\"actor\":\"a\",\"action\":\"state\",\"summary\":\"done\"}\n";
    let path = scratch.join("events.jsonl");
    let run = |body: String, keep: usize| {
        std::fs::write(&path, body).expect("ledger");
        store.retain_events(keep, pins_clean_stop);
        std::fs::read_to_string(&path).expect("trimmed")
    };
    // The newest clean stop fell out of the window: it comes back, alone.
    let dropped = run(
        format!("{}{}{failed}{}", clean(1), clean(2), other.repeat(5)),
        4,
    );
    assert_eq!(dropped, format!("{}{}", clean(2), other.repeat(4)));
    // A clean stop still inside the window needs no pin; nor does a ledger
    // with none at all — that stays byte for byte what retention always did.
    let inside = format!("{}{}{}", clean(1), other.repeat(3), clean(2));
    assert_eq!(
        run(inside.clone(), 4),
        format!("{}{}", other.repeat(3), clean(2))
    );
    assert_eq!(
        run(format!("{failed}{}", other.repeat(6)), 4),
        other.repeat(4)
    );
    assert_eq!(
        run(inside, 10),
        format!("{}{}{}", clean(1), other.repeat(3), clean(2))
    );
}

#[test]
fn only_the_watchdog_cycle_writes_the_beat() {
    let mut callers = Vec::new();
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src").filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if std::fs::read_to_string(&path)
                .is_ok_and(|text| text.contains("watchdog_glue::touch_beat("))
            {
                callers.push(
                    path.file_name()
                        .expect("name")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    assert_eq!(callers, ["watchdog_daemon.rs"], "the beat has one writer");
}
