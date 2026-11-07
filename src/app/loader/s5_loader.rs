//! The real loader Statistics path: cadence, read dependence, scaling and cancellation.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned acceptance fixture IO"
)]
use super::{Answer, Job, Reader, Request};
use crate::stats_store::disk::{self, Glance};
use crate::stats_store::maint::REFRESH;
use crate::stats_store::s5_fixture::{Fixture, S, T, birth, oracle};
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::time::{Duration, Instant};

fn reader(root: &Fixture, now: Instant) -> Reader {
    let mut reader = Reader::new(root.0.clone(), None, None);
    reader.fleet_at = Some(now);
    reader.settings = Some((7, Some(now)));
    reader.settings_session = Some("alpha".into());
    reader.ids.insert("alpha".into(), S.into());
    reader.take(Request::Stats {
        generation: 7,
        shown: true,
    });
    reader
}
fn answer(answer: Option<Answer>) -> Glance {
    match answer {
        Some(Answer::Stats {
            generation: 7,
            glance,
        }) => *glance,
        _ => panic!("matching Statistics answer"),
    }
}
fn ready(glance: Glance) -> crate::stats::View {
    match glance {
        Glance::Ready { view, .. } => view,
        other => panic!("published view, got {other:?}"),
    }
}
fn arm_others(reader: &mut Reader, now: Instant) {
    reader.fleet_at = Some(now);
    reader.settings = Some((7, Some(now)));
}

#[test]
fn s5_loader_refresh_deadline_uses_supplied_clock_and_unchanged_answer_is_silent() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    root.build();
    let now = Instant::now();
    let mut reader = reader(&root, now);
    assert!(
        matches!(reader.next(now), Job::Stats),
        "shown tab starts due"
    );
    assert_eq!(
        ready(answer(reader.stats_read(now))),
        oracle(&dir, "alpha", Some(S))
    );
    let before_due = now + REFRESH - Duration::from_nanos(1);
    arm_others(&mut reader, before_due);
    assert!(
        matches!(reader.next(before_due), Job::Wait(until) if until == now + REFRESH),
        "watch joins the scheduler deadline exactly"
    );
    let due = now + REFRESH;
    arm_others(&mut reader, due);
    assert!(matches!(reader.next(due), Job::Stats));
    assert!(
        reader.stats_read(due).is_none(),
        "unchanged full state emits no answer"
    );
    let before_second = due + REFRESH - Duration::from_nanos(1);
    arm_others(&mut reader, before_second);
    assert!(
        matches!(reader.next(before_second), Job::Wait(until) if until == due + REFRESH),
        "unchanged reads still stamp the injected clock"
    );
    let mut bytes = birth(S, 1);
    bytes.extend(birth(S, 2));
    fs::write(dir.join("events.jsonl"), bytes).expect("new published generation");
    root.build();
    assert_eq!(
        ready(answer(reader.stats_read(due + REFRESH))),
        oracle(&dir, "alpha", Some(S))
    );
}

#[test]
fn s5_loader_same_generation_status_changes_and_session_presence_are_observed() {
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let now = Instant::now();
    let mut reader = reader(&root, now);
    let initial = answer(reader.stats_read(now));
    fs::write(root.0.join("stats/intent"), b"publication is in flight\n")
        .expect("reader judges intent presence, not its contents");
    let updating = answer(reader.stats_read(now + REFRESH));
    match (&initial, &updating) {
        (
            Glance::Ready {
                r#gen: a,
                updating: false,
                ..
            },
            Glance::Ready {
                r#gen: b,
                updating: true,
                ..
            },
        ) => assert_eq!(a, b),
        _ => panic!("same generation changes updating status"),
    }
    fs::remove_file(root.0.join("stats/intent")).expect("publication completed");
    assert_eq!(answer(reader.stats_read(now + REFRESH * 2)), initial);
    let session = root.0.join(format!("stats/session/{S}.view"));
    let valid = fs::read(&session).expect("session view");
    fs::remove_file(&session).expect("session view temporarily unavailable");
    let absent = answer(reader.stats_read(now + REFRESH * 3));
    assert_ne!(
        absent, initial,
        "session presence changes without root generation change"
    );
    fs::write(&session, valid).expect("session restored");
    assert_eq!(answer(reader.stats_read(now + REFRESH * 4)), initial);
}

#[test]
fn s5_loader_hidden_closed_and_stale_generation_never_keep_a_watch_or_read() {
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let now = Instant::now();
    let mut reader = reader(&root, now);
    let _ = answer(reader.stats_read(now));
    for closed in [false, true] {
        reader.take(Request::Stats {
            generation: 7,
            shown: true,
        });
        if closed {
            reader.take(Request::Settings {
                open: false,
                generation: 7,
            });
        } else {
            reader.take(Request::Stats {
                generation: 7,
                shown: false,
            });
        }
        let _ = disk::take_payload_names();
        assert!(reader.stats_read(now + REFRESH).is_none());
        assert!(
            disk::take_payload_names().is_empty(),
            "no background Statistics reads when hidden or closed"
        );
        assert!(!matches!(reader.next(now + REFRESH), Job::Stats));
        reader.take(Request::Settings {
            open: true,
            generation: 7,
        });
        reader.take(Request::Stats {
            generation: 6,
            shown: true,
        });
        assert!(
            reader.stats_read(now + REFRESH * 2).is_none(),
            "stale arm discarded"
        );
    }
}

#[test]
fn s5_loader_statistics_yields_to_due_view_fleet_and_records_jobs() {
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let now = Instant::now();
    let mut reader = reader(&root, now);
    reader
        .dirs
        .insert("alpha".into(), root.0.join("sessions/alpha"));
    reader.rereads.insert("alpha".into());
    assert!(matches!(reader.next(now), Job::View(name) if name == "alpha"));
    reader.rereads.clear();
    reader.looks.insert("alpha".into());
    assert!(matches!(reader.next(now), Job::Look(name) if name == "alpha"));
    reader.looks.clear();
    reader.settings = Some((7, Some(now + super::super::REFRESH)));
    assert!(matches!(
        reader.next(now + super::super::REFRESH),
        Job::Fleet
    ));
    reader.fleet_at = Some(now);
    reader.records = Some(Box::new(super::RecordsAsk {
        generation: 1,
        dialog: crate::app::menu::Dialog::Activity,
        session: "alpha".into(),
        uuid: S.into(),
        dir: root.0.join("sessions/alpha"),
        live: false,
    }));
    assert!(
        matches!(reader.next(now), Job::Records(_)),
        "Activity stays runnable while Statistics is due"
    );
    reader.records = None;
    assert!(
        matches!(reader.next(now), Job::Stats),
        "finite higher-priority work leaves Statistics runnable"
    );
}

#[test]
fn s5_loader_payload_read_set_is_independent_of_all_authoritative_source_content() {
    for unreadable in [false, true] {
        let root = Fixture::new();
        let live = birth(S, 1);
        let archived = birth(T, 2);
        let dir = root.live("alpha", S, &live);
        fs::write(dir.join("memo.tsv"), b"private memo").expect("live memo");
        let archive = root.archive(T, &archived);
        fs::write(archive.join("messages/note"), b"private message").expect("archive message");
        fs::set_permissions(
            archive.join("messages/note"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("private message mode");
        root.build();
        let expected = crate::stats::fold(
            vec![
                crate::stats::Source::live(
                    "alpha",
                    &fs::read(dir.join("meta")).expect("oracle meta"),
                    Ok(live),
                ),
                crate::stats::Source::archive(T, Ok(archived)),
            ],
            Some(S),
        );
        let members = [
            dir.join("meta"),
            dir.join("events.jsonl"),
            dir.join("memo.tsv"),
            archive.join("meta"),
            archive.join("events.jsonl"),
            archive.join("memo.tsv"),
            archive.join("digest.md"),
            archive.join("messages/note"),
        ];
        for member in members {
            fs::rename(&member, member.with_extension("saved"))
                .expect("hide authoritative content");
            if unreadable {
                fs::write(&member, b"opposed unreadable bytes").expect("replacement content");
                fs::set_permissions(&member, fs::Permissions::from_mode(0))
                    .expect("mode 000 replacement");
            }
        }
        let now = Instant::now();
        let mut reader = reader(&root, now);
        let _ = disk::take_payload_names();
        let actual = ready(answer(reader.stats_read(now)));
        let reads = disk::take_payload_names();
        assert_eq!(
            actual, expected,
            "source unavailability never leaks a gap/refusal into the maintained answer"
        );
        let mut names: Vec<_> = reads.iter().map(|(name, _)| name.clone()).collect();
        names.sort();
        let mut expected_names = vec![
            "stats/epoch".to_owned(),
            "stats/view".to_owned(),
            "stats/intent".to_owned(),
            format!("stats/session/{S}.view"),
        ];
        expected_names.sort();
        assert_eq!(names, expected_names);
    }
}

#[test]
fn s5_loader_x1_x4_journal_and_birth_growth_has_identical_view_read_bytes() {
    let mut measured = Vec::new();
    for factor in [1, 4] {
        let root = Fixture::new();
        let mut journal = Vec::new();
        for n in 1..=factor {
            journal.extend(birth(S, n));
        }
        // Dates and fixed-width birth IDs keep bucket cardinality and view digit widths fixed.
        let dir = root.live("alpha", S, &journal);
        root.build();
        let expected = oracle(&dir, "alpha", Some(S));
        assert_eq!(expected.all.total, u64::from(factor));
        let now = Instant::now();
        let mut reader = reader(&root, now);
        let _ = disk::take_payload_names();
        assert_eq!(ready(answer(reader.stats_read(now))), expected);
        let reads = disk::take_payload_names();
        assert!(!reads.is_empty(), "counter must observe real reads");
        measured.push((journal.len(), reads));
    }
    assert_eq!(
        measured[1].0,
        measured[0].0 * 4,
        "fixture really grows source bytes fourfold"
    );
    assert_eq!(
        measured[0].1, measured[1].1,
        "fixed bucket/cardinality view reads do not scale with history"
    );
}

#[test]
fn s5_loader_roots_never_mix() {
    let a = Fixture::new();
    let adir = a.live("alpha", S, &birth(S, 1));
    a.build();
    let b = Fixture::new();
    let bdir = b.live("alpha", T, &birth(T, 2));
    b.build();
    let now = Instant::now();
    let mut ar = reader(&a, now);
    let mut br = reader(&b, now);
    let av = ready(answer(ar.stats_read(now)));
    let bv = ready(answer(br.stats_read(now)));
    assert_eq!(av.all, oracle(&adir, "alpha", None).all);
    assert_eq!(bv.all, oracle(&bdir, "alpha", None).all);
    assert_ne!(
        av.this, bv.this,
        "same session name in two roots cannot borrow scope data"
    );
}
