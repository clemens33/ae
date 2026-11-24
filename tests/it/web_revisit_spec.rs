//! S2 desk and topic contracts over the authenticated listener.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::{Listener, VERSION, array, bearer, get, property};
use ae::json::Value;
use ae::web::clock::ManualClock;
use ae::web::server::{NoProbe, Options, Running, start};
use ae::web::state::{Read, Source};
use ae::web::topics::{self, Sticky};

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const OLD: &str = "2026-10-01T08:00:00Z";
const TIED: &str = "2026-10-02T08:00:00Z";
const NEW: &str = "2026-10-03T08:00:00Z";
const KEPT: &str = "2026-10-04T08:00:00Z";

struct DeskSource(PathBuf);

impl Source for DeskSource {
    fn read(&self) -> Read {
        Read {
            desk: topics::collect(&self.0, &mut Sticky::default()),
            ..Read::default()
        }
    }
}

fn start_desk(root: &Path, clock: &Arc<ManualClock>) -> Listener {
    Listener(Some(
        start(Options {
            port: 0,
            clock: clock.clone(),
            source: Arc::new(DeskSource(root.to_path_buf())),
            probe: Arc::new(NoProbe),
            links: Arc::new(|_| {}),
            version: VERSION.into(),
        })
        .expect("start the wire fixture"),
    ))
}

fn listening(listener: &Listener) -> &Running {
    listener.0.as_ref().expect("running listener")
}

fn seat(root: &Path) -> PathBuf {
    let place = root.join("sessions/s2-wire");
    let overlay = root.join("orchestrator.config");
    let socket = root.join("no-server");
    std::fs::create_dir_all(&place).expect("make seat directory");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("write seat overlay");
    std::fs::write(
        place.join("meta"),
        format!(
            "schema=2\nsession=s2-wire\nsession_id={UUID}\nlayout=lead-pair\nseat.main=lead\n\
             meta_agent=true\nlocal_config={}\ntmux_server_kind=socket\ntmux_server={}\n",
            overlay.display(),
            socket.display(),
        ),
    )
    .expect("write stopped seat meta");
    place
}

fn slugs(rows: &[Value]) -> Vec<&str> {
    rows.iter()
        .map(|row| row.get_str("slug").expect("a slug in each row"))
        .collect()
}

fn rows<'a>(desk: &'a Value, name: &str) -> &'a [Value] {
    let group = array(desk, "groups")
        .iter()
        .find(|group| group.get_str("group") == Some(name))
        .expect("named desk group");
    array(group, "rows")
}

fn assert_entry(
    actual: &Value,
    slug: &str,
    title: Option<&str>,
    line: Option<&str>,
    reviewed: &str,
    next: &str,
) {
    assert_object(
        actual,
        &[
            ("slug", Value::str(slug)),
            ("title", title.map_or(Value::Null, Value::str)),
            ("line", line.map_or(Value::Null, Value::str)),
            ("reviewed", Value::str(reviewed)),
            ("next", Value::str(next)),
        ],
    );
}

fn assert_object(actual: &Value, expected: &[(&str, Value)]) {
    let Value::Obj(fields) = actual else {
        panic!("expected an object: {actual:?}");
    };
    assert_eq!(fields.len(), expected.len(), "{actual:?}");
    for (name, value) in expected {
        assert_eq!(actual.get(name), Some(value), "{name} in {actual:?}");
    }
}

fn append(place: &Path, memo: &str) {
    ae::store::open(place)
        .append_memo(memo.as_bytes())
        .expect("append topic memo");
}

fn populate(place: &Path) {
    append(
        place,
        &format!(
            "2026-10-20T08:00:00Z\tlead\ttopic-zeta\tstate: quiet | what: Summary | last: Previous\n\
             {OLD}\tlead\ttopic-zeta\ttitle: Oldest | next: Plan \\| check\n\
             {NEW}\tlead\ttopic-alpha\tstate: quiet | what: Alpha summary\n\
             {TIED}\tlead\ttopic-delta\tstate: quiet | last: Delta last | what: Delta summary\n\
             {TIED}\tlead\ttopic-beta\tstate: quiet\n\
             zzz\tlead\ttopic-zz-bad\tstate: quiet\n\
             \tlead\ttopic-z-empty\tstate: quiet\n\
             {OLD}\tlead\ttopic-waited\tstate: quiet\n"
        ),
    );
    for (group, state) in [
        ("need", "needs"),
        ("moving", "moving"),
        ("parked", "parked"),
        ("closed", "closed"),
        ("unreadable", "broken"),
    ] {
        append(
            place,
            &format!(
                "{NEW}\tlead\ttopic-a-{group}\tstate: {state}\n\
                 {OLD}\tlead\ttopic-z-{group}\tstate: {state}\n"
            ),
        );
    }
    let board = ae::desk::parse_input(b"DECIDE\nD1 | Choose the next #waited checkpoint\n")
        .expect("parse the open decision");
    ae::deskstore::set(
        place,
        board,
        ae::time::Timestamp::from_epoch(1_795_000_000),
        "lead",
    )
    .expect("publish decision tagged to an otherwise quiet topic");
}

#[test]
fn desk_orders_only_quiet_by_review_time_and_exposes_one_revisit_cycle() {
    let scratch = crate::cli::OwnedScratch::root("wrev", "order");
    let place = seat(scratch.path());
    populate(&place);
    let clock = Arc::new(ManualClock::new(1_795_000_000));
    let listener = start_desk(scratch.path(), &clock);
    let running = listening(&listener);
    let host = running.addr();
    let token = bearer(running);
    let auth = Some(token.as_str());

    let first = get(host, "/api/state", auth, None);
    assert_eq!(first.status, 200);
    let tag = first.etag.as_deref().expect("state ETag");
    let state = first.json();
    let desk = property(&state, "desk");
    let quiet = ["z-empty", "zz-bad", "zeta", "beta", "delta", "alpha"];
    assert_eq!(slugs(rows(desk, "quiet")), quiet);
    let revisit = array(desk, "revisit");
    assert_eq!(slugs(revisit), quiet);
    for (actual, (slug, title, line, reviewed, next)) in revisit.iter().zip([
        ("z-empty", None, None, "", "zz-bad"),
        ("zz-bad", None, None, "zzz", "zeta"),
        ("zeta", Some("Oldest"), Some("Plan | check"), OLD, "beta"),
        ("beta", None, None, TIED, "delta"),
        ("delta", None, Some("Delta last"), TIED, "alpha"),
        ("alpha", None, Some("Alpha summary"), NEW, "z-empty"),
    ]) {
        assert_entry(actual, slug, title, line, reviewed, next);
    }
    for (name, expected) in [
        ("needs_you", ["a-need", "waited", "z-need"].as_slice()),
        ("moving", ["a-moving", "z-moving"].as_slice()),
        ("parked", ["a-parked", "z-parked"].as_slice()),
        ("closed", ["a-closed", "z-closed"].as_slice()),
        ("unreadable", ["a-unreadable", "z-unreadable"].as_slice()),
    ] {
        assert_eq!(slugs(rows(desk, name)), expected, "{name}");
    }
    assert_eq!(get(host, "/api/state", auth, Some(tag)).status, 304);

    append(&place, &format!("{KEPT}\tlead\ttopic-zeta\treview: keep\n"));
    clock.advance(Duration::from_secs(1));
    let changed = get(host, "/api/state", auth, Some(tag));
    assert_eq!(changed.status, 200);
    assert_ne!(changed.etag.as_deref(), Some(tag));
    let state = changed.json();
    let desk = property(&state, "desk");
    let ordered = ["z-empty", "zz-bad", "beta", "delta", "alpha", "zeta"];
    assert_eq!(slugs(rows(desk, "quiet")), ordered);
    let revisit = array(desk, "revisit");
    assert_eq!(slugs(revisit), ordered);
    assert_eq!(revisit[1].get_str("next"), Some("beta"));
    assert_eq!(revisit[4].get_str("next"), Some("zeta"));
    assert_entry(
        &revisit[5],
        "zeta",
        Some("Oldest"),
        Some("Plan | check"),
        KEPT,
        "z-empty",
    );
}

#[test]
fn topic_document_supplies_verdicts_only_for_close_eligible_cards() {
    let scratch = crate::cli::OwnedScratch::root("wrev", "verdict");
    populate(&seat(scratch.path()));
    let clock = Arc::new(ManualClock::new(1_795_000_000));
    let listener = start_desk(scratch.path(), &clock);
    let running = listening(&listener);
    let token = bearer(running);
    for (slug, group, eligible) in [
        ("zeta", "quiet", true),
        ("a-moving", "moving", true),
        ("a-parked", "parked", true),
        ("a-need", "needs_you", false),
        ("waited", "needs_you", false),
        ("a-closed", "closed", false),
        ("a-unreadable", "unreadable", false),
    ] {
        let path = format!("/api/topic/{slug}");
        let response = get(running.addr(), &path, Some(&token), None);
        assert_eq!(response.status, 200, "{slug}");
        let card = response.json();
        assert_eq!(card.get_str("group"), Some(group), "{slug}");
        let verdict = property(&card, "verdict");
        if eligible {
            assert_object(
                verdict,
                &[
                    ("keep", Value::str(format!("#{slug} keep"))),
                    ("ask", Value::str(format!("#{slug} ask"))),
                    ("close", Value::str(format!("#{slug} close "))),
                    ("reason_max", Value::Num(200)),
                ],
            );
        } else {
            assert_eq!(verdict, &Value::Null, "{slug}");
        }
    }
}

#[test]
fn an_empty_desk_has_no_revisit_and_a_single_topic_points_to_itself() {
    let scratch = crate::cli::OwnedScratch::root("wrev", "singleton");
    let clock = Arc::new(ManualClock::new(1_795_000_000));
    let listener = start_desk(scratch.path(), &clock);
    let running = listening(&listener);
    let token = bearer(running);
    let read = || {
        let response = get(running.addr(), "/api/state", Some(&token), None);
        assert_eq!(response.status, 200);
        response.json()
    };

    assert!(array(property(&read(), "desk"), "revisit").is_empty());
    let long = "🦊".repeat(201);
    append(
        &seat(scratch.path()),
        &format!("{OLD}\tlead\ttopic-solo\tstate: quiet | last: {long}\n"),
    );
    clock.advance(Duration::from_secs(1));
    let value = read();
    let revisit = array(property(&value, "desk"), "revisit");
    assert_eq!(revisit.len(), 1);
    assert_entry(
        &revisit[0],
        "solo",
        None,
        Some(&"🦊".repeat(200)),
        OLD,
        "solo",
    );
}
