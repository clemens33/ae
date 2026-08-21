//! Phase 2 end to end: phase-1 facts in, classified liveness and a
//! schema-version-2 digest out.
//!
//! Gate: `docs/migration/p1-phase2-gate.md`, blob
//! `29db943aa85319534301332052105ba16df03b4d`. Each test names the criterion it
//! answers. The unit-level halves live beside their code in `src/liveness.rs`
//! and `src/tmux.rs`; what is here is what needs a real filesystem, a real tmux,
//! or the whole pipeline at once.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use ae::digest::Status;
use ae::inventory::{
    Candidate, DiscoveredSession, Discovery, FailedSource, Inventory, LiveSighting, MetaRead,
    QueryFailed, Roots, ServerId, durable_records, take,
};
use ae::json;
use ae::listing::{render, world_of};
use ae::liveness::{Snapshot, classify};
use ae::meta::{Selector, ServerSelector};
use ae::session::DEFAULT_UNANSWERED_SECS;
use ae::time::Timestamp;

use super::parity::Invocation;
use super::parity::capture::ExitOutcome;
use super::parity::capture::raw;

const NOW: Timestamp = Timestamp::from_epoch(1_780_000_000);

fn named(server: &str) -> ServerId {
    ServerId::Selected(Selector::Name(server.to_owned()))
}

fn positive(server: &str) -> ServerSelector {
    ServerSelector::Positive(Selector::Name(server.to_owned()))
}

/// What one calibrated backend response was, recorded field by field.
///
/// Criterion 20: target server, exact returned names, ownership result and
/// transport success/failure are recorded SEPARATELY. A recorder that collapsed
/// success and failure into one shape could not tell the gate anything.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    target: String,
    transport_succeeded: bool,
    names: Vec<String>,
    ownership: Vec<(String, bool)>,
}

struct Recorder {
    worlds: Vec<(ServerId, Result<Vec<DiscoveredSession>, QueryFailed>)>,
    log: RefCell<Vec<Record>>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            worlds: Vec::new(),
            log: RefCell::new(Vec::new()),
        }
    }

    fn live(mut self, server: ServerId, sessions: &[(&str, Option<&str>)]) -> Self {
        self.worlds.push((
            server,
            Ok(sessions
                .iter()
                .map(|(name, marker)| DiscoveredSession {
                    name: (*name).to_owned(),
                    marker: marker.map(ToOwned::to_owned),
                })
                .collect()),
        ));
        self
    }

    fn down(mut self, server: ServerId) -> Self {
        self.worlds.push((server, Err(QueryFailed)));
        self
    }

    fn records(&self) -> Vec<Record> {
        self.log.borrow().clone()
    }

    fn targets(&self) -> Vec<String> {
        let mut seen: Vec<String> = self
            .log
            .borrow()
            .iter()
            .map(|record| record.target.clone())
            .collect();
        seen.sort();
        seen.dedup();
        seen
    }
}

fn spell(server: &ServerId) -> String {
    match server {
        ServerId::Ambient => "ambient".to_owned(),
        ServerId::Selected(Selector::Name(name)) => format!("name:{name}"),
        ServerId::Selected(Selector::Socket(path)) => format!("socket:{}", path.display()),
    }
}

impl Discovery for Recorder {
    /// **HAZARD:** an UNREGISTERED server answers `Ok(vec![])` — a successful
    /// EMPTY query, which is what proves a name absent and reaches `stopped`.
    /// Fixtures depend on that, so the fallback stays; the cost is that a
    /// MISTYPED server name is indistinguishable from a registered empty one,
    /// and a test asserting `stopped` passes with the typo while proving nothing
    /// about the server it meant. Where the claim depends on WHICH server
    /// answered, assert `targets()` as well.
    fn enumerate(&self, server: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed> {
        let answer = self
            .worlds
            .iter()
            .find(|(known, _)| known == server)
            .map_or(Ok(Vec::new()), |(_, answer)| answer.clone());
        self.log.borrow_mut().push(Record {
            target: spell(server),
            transport_succeeded: answer.is_ok(),
            names: answer
                .as_ref()
                .map(|sessions| sessions.iter().map(|s| s.name.clone()).collect())
                .unwrap_or_default(),
            ownership: answer
                .as_ref()
                .map(|sessions| {
                    sessions
                        .iter()
                        .map(|s| {
                            (
                                s.name.clone(),
                                ae::liveness::positively_owned(&s.name, s.marker.as_deref()),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        });
        answer
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-phase2-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "a scratch dir must be creatable"
        );
        Self(dir)
    }

    /// A session whose meta is READABLE, names a positive server selector, and
    /// carries a roster — the product-valid not-degraded shape.
    fn healthy(&self, name: &str, extra: &str) -> PathBuf {
        self.write(
            name,
            &format!(
                "mode=local\ntmux_server_kind=name\ntmux_server=B\nagent.main=cl:lead\n{extra}"
            ),
        )
    }

    /// A session whose meta is READABLE and names a positive server selector,
    /// but has NO ROSTER — SC-405k/SC-405i record loss.
    ///
    /// This is the pairing the phase-2 gate's criterion 13 needs and a
    /// hand-built struct cannot honestly supply: degradation from an
    /// INDEPENDENT record fact, sitting beside a selector positive enough to
    /// reach a real liveness answer. An unreadable meta cannot do it — that
    /// yields a `missing` selector by construction, so `running`+degraded would
    /// be unreachable and any fixture asserting it would be describing a state
    /// the product can never produce.
    fn degraded_but_addressable(&self, name: &str) -> PathBuf {
        self.write(name, "mode=local\ntmux_server_kind=name\ntmux_server=B\n")
    }

    /// A session directory whose meta is absent — SC-405i record loss, and a
    /// `missing` selector.
    fn no_meta(&self, name: &str) -> PathBuf {
        let dir = self.0.join("sessions").join(name);
        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "a session fixture must be creatable"
        );
        dir
    }

    /// A session whose meta is readable and carries a roster but NO selector.
    fn no_selector(&self, name: &str) -> PathBuf {
        self.write(name, "mode=local\nagent.main=cl:lead\n")
    }

    /// Write an event log beside a session's meta.
    ///
    /// SC-519 makes an ABSENT log a quiet stream, which is why a differential
    /// test whose fixtures have no log is blind to an event reread: absent
    /// before, absent after, absent once the tree is gone — the arm never
    /// varies, so nothing about it can be discriminated.
    fn events(&self, name: &str, lines: &[String]) -> PathBuf {
        let dir = self.0.join("sessions").join(name);
        let mut body = lines.join("\n");
        body.push('\n');
        let path = dir.join("events.jsonl");
        assert!(
            fs::write(&path, body).is_ok(),
            "an event log must be writable"
        );
        path
    }

    fn write(&self, name: &str, meta: &str) -> PathBuf {
        let dir = self.0.join("sessions").join(name);
        let written = fs::create_dir_all(&dir).and_then(|()| fs::write(dir.join("meta"), meta));
        assert!(written.is_ok(), "a session fixture must be writable");
        dir
    }

    /// Every candidate, discovered by the PRODUCT reader.
    ///
    /// Nothing in this file hand-builds a `DurableRecord` any more: a fixture
    /// can succeed at constructing a state the product could never produce, and
    /// then every assertion about it passes while proving nothing.
    fn candidates(&self) -> Vec<Candidate> {
        take(durable_records(&self.roots()), None, &Recorder::new()).candidates
    }

    fn roots(&self) -> Roots {
        Roots::under(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// One event line, in the shape SC-510a/SC-510c define.
fn event(ts: &str, actor: &str, action: &str, extra: &str) -> String {
    format!(r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}"{extra}}}"#)
}

/// The product-discovered candidate called `name`.
fn pick(candidates: &[Candidate], name: &str) -> Candidate {
    match candidates.iter().find(|candidate| candidate.name == name) {
        Some(candidate) => candidate.clone(),
        None => panic!("the reader did not discover {name}"),
    }
}

fn inventory(candidates: Vec<Candidate>) -> Inventory {
    Inventory {
        candidates,
        incomplete: Vec::new(),
    }
}

fn digest_of(snapshot: &Snapshot) -> json::Value {
    let world = world_of(snapshot, NOW, DEFAULT_UNANSWERED_SECS);
    let rendered = render(&args(&["--all", "--json"]), &world);
    match json::parse(rendered.trim_end()) {
        Ok(document) => document,
        Err(why) => panic!("the digest must be one complete document: {why:?}"),
    }
}

fn args(tokens: &[&str]) -> ae::filters::ListArgs {
    match ae::filters::ListArgs::parse(tokens) {
        Ok(parsed) => parsed,
        Err(unknown) => panic!("these are documented flags: {unknown:?}"),
    }
}

fn sessions_of(document: &json::Value) -> Vec<(String, String, bool)> {
    let Some(json::Value::Arr(entries)) = document.get("sessions") else {
        panic!("sessions must be an array");
    };
    entries
        .iter()
        .map(|entry| {
            (
                match entry.get_str("name") {
                    Some(name) => name.to_owned(),
                    None => panic!("every session entry carries a name"),
                },
                match entry.get_str("status") {
                    Some(status) => status.to_owned(),
                    None => panic!("every session entry carries a status"),
                },
                entry.get("degraded") == Some(&json::Value::Bool(true)),
            )
        })
        .collect()
}

// ---- criterion 20: the recorder is calibrated before anything trusts it ----

#[test]
fn criterion_20_the_recorder_tells_success_absence_ownership_and_failure_apart() {
    let backend = Recorder::new()
        .live(named("empty"), &[])
        .live(named("owned"), &[("alpha", Some("alpha"))])
        .live(named("unowned"), &[("alpha", None)])
        .down(named("broken"));
    for server in ["empty", "owned", "unowned", "broken"] {
        let _ = backend.enumerate(&named(server));
    }

    let records = backend.records();
    assert_eq!(
        records,
        vec![
            Record {
                target: "name:empty".to_owned(),
                transport_succeeded: true,
                names: Vec::new(),
                ownership: Vec::new(),
            },
            Record {
                target: "name:owned".to_owned(),
                transport_succeeded: true,
                names: vec!["alpha".to_owned()],
                ownership: vec![("alpha".to_owned(), true)],
            },
            Record {
                target: "name:unowned".to_owned(),
                transport_succeeded: true,
                names: vec!["alpha".to_owned()],
                ownership: vec![("alpha".to_owned(), false)],
            },
            Record {
                target: "name:broken".to_owned(),
                transport_succeeded: false,
                names: Vec::new(),
                ownership: Vec::new(),
            },
        ],
        "all four controls land, and success never records the same shape as failure"
    );
    assert_ne!(
        records[0], records[3],
        "successful-empty and failure are the pair a weak recorder collapses"
    );
}

// ---- criterion 1: the complete inventory precedes classification -----------

#[test]
fn criterion_1_the_complete_inventory_is_what_classification_receives() {
    let scratch = Scratch::new("boundary");
    scratch.healthy("alpha", "");
    scratch.healthy("beta", "");
    let backend = Recorder::new();

    let taken = take(durable_records(&scratch.roots()), None, &backend);
    let at_inventory: Vec<String> = taken
        .candidates
        .iter()
        .map(|candidate| candidate.name.clone())
        .collect();

    let snapshot = classify(taken, &backend);
    let at_classify: Vec<String> = snapshot
        .sessions
        .iter()
        .map(|classified| classified.candidate.name.clone())
        .collect();

    assert_eq!(at_inventory, at_classify, "the two semantic sets are equal");
    assert_eq!(at_classify.len(), 2, "and nothing was materialized inside");
}

// ---- criterion 14: NO phase-2 read, at either boundary ---------------------

#[test]
fn criterion_14_the_whole_phase_survives_the_filesystem_disappearing_after_inventory() {
    // The obligation is on PHASE 2, which the gate defines as the classified set
    // PLUS the successor digest. So the roots go away after `inventory
    // complete`, and BOTH boundaries must still answer — classification and
    // emission. An earlier version of this test called only `classify`, and a
    // whole-phase obligation checked on one function is how the second read
    // survived review.
    let scratch = Scratch::new("no-reread");
    scratch.healthy("alpha", "origin=/o\ngoal=ship it\n");
    scratch.no_meta("damaged");
    let taken = take(durable_records(&scratch.roots()), None, &Recorder::new());

    assert!(
        fs::remove_dir_all(scratch.0.join("sessions")).is_ok(),
        "the roots go away"
    );

    let backend = Recorder::new().live(named("B"), &[("alpha", Some("alpha"))]);
    let snapshot = classify(taken, &backend);
    let document = digest_of(&snapshot);

    assert_eq!(
        backend.targets(),
        ["name:B"],
        "one backend query, no rescan"
    );
    let mut emitted = sessions_of(&document);
    emitted.sort();
    assert_eq!(
        emitted,
        vec![
            ("alpha".to_owned(), "running".to_owned(), false),
            ("damaged".to_owned(), "unknown".to_owned(), true),
        ],
        "every SC-509 fact came from the snapshot phase 1 captured"
    );
    let Some(json::Value::Arr(entries)) = document.get("sessions") else {
        panic!("sessions must be an array");
    };
    let Some(alpha) = entries.iter().find(|e| e.get_str("name") == Some("alpha")) else {
        panic!("alpha must be emitted");
    };
    assert_eq!(
        alpha.get_str("goal"),
        Some("ship it"),
        "including the meta fields, which no second read could have supplied"
    );
}

#[test]
fn criterion_9_a_tmux_only_candidate_leaves_the_filesystem_untouched() {
    let scratch = Scratch::new("no-writes");
    scratch.healthy("elsewhere", "");
    let before = manifest(&scratch.0);

    let candidate = Candidate::tmux_only(LiveSighting {
        server: named("A"),
        name: "ghost".to_owned(),
        marker: "ghost".to_owned(),
    });
    let backend = Recorder::new().down(named("A"));
    let snapshot = classify(inventory(vec![candidate]), &backend);
    let _ = digest_of(&snapshot);

    assert_eq!(
        snapshot.sessions[0].status,
        Status::Running,
        "the snapshot fact stands even though A is unavailable"
    );
    assert!(snapshot.sessions[0].candidate.durable.is_none());
    assert_eq!(
        manifest(&scratch.0),
        before,
        "transient discovery evidence was not persisted as durable state"
    );
}

/// Every path under `root`, with its kind and content length.
fn manifest(root: &Path) -> Vec<String> {
    let mut seen = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                seen.push(format!("dir {}", path.display()));
                stack.push(path);
            } else {
                let len = fs::read(&path).map(|bytes| bytes.len()).unwrap_or_default();
                seen.push(format!("file {} {len}", path.display()));
            }
        }
    }
    seen.sort();
    seen
}

// ---- criterion 13: unknown and degraded are orthogonal, through the digest --

/// The criterion-13 matrix, built entirely by the PRODUCT reader.
///
/// Every degraded cell earns its degradation from an independent record fact —
/// a readable meta with a positive selector and no roster (SC-405k/SC-405i), or
/// an absent meta — never from breaking the liveness query, and never from a
/// hand-assembled struct pairing a positive selector with a failed read, which
/// `record_at` cannot produce and the product therefore never emits.
fn orthogonality_fixture(scratch: &Scratch) -> Vec<Candidate> {
    scratch.healthy("running-clean", "");
    scratch.degraded_but_addressable("running-damaged");
    scratch.no_selector("unknown-clean");
    scratch.no_meta("unknown-damaged");
    scratch.healthy("stopped-clean", "");
    scratch.degraded_but_addressable("stopped-damaged");
    scratch.candidates()
}

fn orthogonality_backend() -> Recorder {
    Recorder::new().live(
        named("B"),
        &[
            ("running-clean", Some("running-clean")),
            ("running-damaged", Some("running-damaged")),
        ],
    )
}

#[test]
fn criterion_13_unknown_and_degraded_are_independent_at_both_boundaries() {
    let scratch = Scratch::new("orthogonal");
    let candidates = orthogonality_fixture(&scratch);
    // Every cell is reachable: the product reader produced each one.
    for name in [
        "running-clean",
        "running-damaged",
        "unknown-clean",
        "unknown-damaged",
        "stopped-clean",
        "stopped-damaged",
    ] {
        let _ = pick(&candidates, name);
    }

    let snapshot = classify(inventory(candidates), &orthogonality_backend());

    let mut classified: Vec<(String, Status)> = snapshot
        .sessions
        .iter()
        .map(|entry| (entry.candidate.name.clone(), entry.status))
        .collect();
    classified.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        classified,
        vec![
            ("running-clean".to_owned(), Status::Running),
            ("running-damaged".to_owned(), Status::Running),
            ("stopped-clean".to_owned(), Status::Stopped),
            ("stopped-damaged".to_owned(), Status::Stopped),
            ("unknown-clean".to_owned(), Status::Unknown),
            ("unknown-damaged".to_owned(), Status::Unknown),
        ],
        "flipping degradation alone never moved a status"
    );

    let mut emitted = sessions_of(&digest_of(&snapshot));
    emitted.sort();
    assert_eq!(
        emitted,
        vec![
            ("running-clean".to_owned(), "running".to_owned(), false),
            ("running-damaged".to_owned(), "running".to_owned(), true),
            ("stopped-clean".to_owned(), "stopped".to_owned(), false),
            ("stopped-damaged".to_owned(), "stopped".to_owned(), true),
            ("unknown-clean".to_owned(), "unknown".to_owned(), false),
            ("unknown-damaged".to_owned(), "unknown".to_owned(), true),
        ],
        "every combination survives serialization, and every one is product-reachable"
    );
}

#[test]
fn criterion_13_the_degraded_cells_are_reachable_rather_than_asserted() {
    // The control for the fixture itself. `running`+degraded is only meaningful
    // if the reader can produce a record that is BOTH addressable and damaged;
    // if it could not, the matrix above would be describing an impossible state.
    let scratch = Scratch::new("reachable");
    scratch.degraded_but_addressable("both");
    let candidate = pick(&scratch.candidates(), "both");
    let Some(record) = &candidate.durable else {
        panic!("a durable candidate");
    };
    assert_eq!(
        record.server,
        positive("B"),
        "the reader derived a POSITIVE selector, so this record can reach a liveness answer"
    );
    assert_eq!(
        record.meta_read,
        MetaRead::Parsed,
        "from a meta it could read — the damage is a separate record fact"
    );
    // A fresh recorder, not the matrix one with another entry appended: the
    // double answers with the FIRST world it holds for a server, so a second
    // `live` for the same server is shadowed and the fixture would silently
    // describe an empty B.
    let snapshot = classify(
        inventory(vec![candidate]),
        &Recorder::new().live(named("B"), &[("both", Some("both"))]),
    );
    assert_eq!(snapshot.sessions[0].status, Status::Running);
    assert_eq!(
        sessions_of(&digest_of(&snapshot)),
        vec![("both".to_owned(), "running".to_owned(), true)],
        "running AND degraded, produced end to end"
    );
}

// ---- criterion 16: every successor digest, every shape --------------------

#[test]
fn criterion_16_every_successor_digest_is_version_2_with_a_closed_status_domain() {
    let scratch = Scratch::new("matrix");
    scratch.healthy("r", "");
    scratch.healthy("s", "");
    scratch.no_selector("u");
    let all = scratch.candidates();
    let matrix = Scratch::new("matrix-degraded");
    let degradation = orthogonality_fixture(&matrix);

    // ONE server response carrying every exact name any shape needs. The
    // recorder answers with the FIRST world it holds for a server, so appending
    // a second `live` for B — which is what this loop used to do — is SHADOWED:
    // the running-only shape classified `stopped` and the mixed shape was
    // stopped/stopped/unknown, and a closed-domain check cannot see that,
    // because `stopped` is in the domain. The file records that first-answer
    // rule a few tests up; knowing it and applying it where it mattered turned
    // out to be different acts.
    let world = || {
        Recorder::new().live(
            named("B"),
            &[
                ("r", Some("r")),
                ("running-clean", Some("running-clean")),
                ("running-damaged", Some("running-damaged")),
            ],
        )
    };

    // Each shape names the statuses it must CONTAIN. A shape that does not hold
    // what its name says is not that shape, and the domain check below would
    // pass either way.
    let shapes: Vec<(&str, Vec<Candidate>, Vec<&str>)> = vec![
        ("empty", Vec::new(), Vec::new()),
        ("running only", vec![pick(&all, "r")], vec!["running"]),
        ("stopped only", vec![pick(&all, "s")], vec!["stopped"]),
        ("unknown only", vec![pick(&all, "u")], vec!["unknown"]),
        (
            "mixed",
            vec![pick(&all, "r"), pick(&all, "s"), pick(&all, "u")],
            vec!["running", "stopped", "unknown"],
        ),
        (
            "degradation matrix",
            degradation,
            vec![
                "running", "running", "stopped", "stopped", "unknown", "unknown",
            ],
        ),
    ];

    for (shape, candidates, expected) in shapes {
        for complete in [true, false] {
            let backend = world();
            let snapshot = classify(
                Inventory {
                    candidates: candidates.clone(),
                    incomplete: if complete {
                        Vec::new()
                    } else {
                        vec![FailedSource::CanonicalRoot(PathBuf::from("/gone"))]
                    },
                },
                &backend,
            );
            let rendered = render(
                &args(&["--all", "--json"]),
                &world_of(&snapshot, NOW, DEFAULT_UNANSWERED_SECS),
            );
            let document = match json::parse(rendered.trim_end()) {
                Ok(document) => document,
                Err(why) => panic!("{shape}/{complete}: valid JSON: {why:?}"),
            };

            // THE SHAPE IS WHAT IT CLAIMS, asserted before anything else is
            // read off it.
            let mut emitted: Vec<String> = sessions_of(&document)
                .into_iter()
                .map(|(_, status, _)| status)
                .collect();
            emitted.sort();
            let mut wanted: Vec<String> =
                expected.iter().map(|status| (*status).to_owned()).collect();
            wanted.sort();
            assert_eq!(
                emitted, wanted,
                "{shape}/{complete}: the fixture does not contain the statuses its name claims"
            );

            assert_eq!(
                document.get("schema_version"),
                Some(&json::Value::Num(2)),
                "{shape}/{complete}: numeric version 2, unconditionally"
            );
            assert_eq!(
                rendered.matches("\"schema_version\"").count(),
                1,
                "{shape}/{complete}: exactly once"
            );
            assert_eq!(
                document.get("inventory_complete"),
                Some(&json::Value::Bool(complete)),
                "{shape}/{complete}: the supplied boolean, not a derived one"
            );
            assert_eq!(
                rendered.matches("\"inventory_complete\"").count(),
                1,
                "{shape}/{complete}: exactly once"
            );
            for (name, status, _) in sessions_of(&document) {
                assert!(
                    ["running", "unknown", "stopped"].contains(&status.as_str()),
                    "{shape}/{complete}: {name} carried {status}, outside the closed domain"
                );
            }
        }
    }
}

#[test]
fn criterion_19_no_emitted_document_pairs_version_1_with_unknown() {
    let scratch = Scratch::new("v1-never-unknown");
    scratch.no_selector("u");
    let snapshot = classify(inventory(scratch.candidates()), &Recorder::new());
    for flags in [
        vec!["--json"],
        vec!["--all", "--json"],
        vec!["--stopped", "--json"],
    ] {
        let world = world_of(&snapshot, NOW, DEFAULT_UNANSWERED_SECS);
        let rendered = render(&args(&flags), &world);
        assert!(
            !rendered.contains("\"schema_version\":1"),
            "{flags:?}: no successor path emits version 1"
        );
        if rendered.contains("\"status\":\"unknown\"") {
            assert!(
                rendered.contains("\"schema_version\":2"),
                "{flags:?}: unknown only ever appears under version 2"
            );
        }
    }
}

// ---- criterion 15: the render path cannot change knowledge -----------------

#[test]
fn criterion_15_no_filter_or_rendering_route_changes_a_classified_status() {
    let scratch = Scratch::new("render-invariance");
    scratch.healthy("r", "");
    scratch.healthy("s", "");
    scratch.no_selector("u");
    let backend = Recorder::new().live(named("B"), &[("r", Some("r"))]);
    let snapshot = classify(inventory(scratch.candidates()), &backend);

    let classified: Vec<(String, Status)> = snapshot
        .sessions
        .iter()
        .map(|entry| (entry.candidate.name.clone(), entry.status))
        .collect();

    for flags in [
        vec!["--json"],
        vec!["--all", "--json"],
        vec!["--stopped", "--json"],
        vec!["--needs-attn", "--all", "--json"],
    ] {
        let world = world_of(&snapshot, NOW, DEFAULT_UNANSWERED_SECS);
        let document = match json::parse(render(&args(&flags), &world).trim_end()) {
            Ok(document) => document,
            Err(why) => panic!("{flags:?}: one document: {why:?}"),
        };
        for (name, status, _) in sessions_of(&document) {
            let expected = match classified.iter().find(|(known, _)| *known == name) {
                Some((_, status)) => *status,
                None => panic!("{name} was rendered but never classified"),
            };
            assert_eq!(
                status,
                expected.as_str(),
                "{flags:?}: {name} was rendered with a different status than it was classified"
            );
        }
    }

    let world = world_of(&snapshot, NOW, DEFAULT_UNANSWERED_SECS);
    let table = render(&args(&["--all"]), &world);
    for (name, status) in &classified {
        assert!(
            table.contains(status.as_str()),
            "the human rendering of {name} lost its status"
        );
    }
}

// ---- criterion 17: version 2 preserves the rest of SC-509 ------------------

#[test]
fn criterion_17_the_version_bump_changes_the_version_and_the_status_domain_only() {
    let scratch = Scratch::new("preserved");
    scratch.healthy(
        "my-feature",
        "origin=/o\nwork_dir=/w\ngoal=ship the login flow\n",
    );
    let snapshot = classify(
        inventory(scratch.candidates()),
        &Recorder::new().live(named("B"), &[("my-feature", Some("my-feature"))]),
    );
    let document = digest_of(&snapshot);

    assert_eq!(document.get("schema_version"), Some(&json::Value::Num(2)));
    assert!(document.get("generated_at").is_some(), "SC-509 field kept");
    let Some(json::Value::Arr(entries)) = document.get("sessions") else {
        panic!("sessions must be an array");
    };
    let entry = &entries[0];
    assert_eq!(entry.get_str("name"), Some("my-feature"));
    assert_eq!(entry.get_str("status"), Some("running"));
    assert_eq!(entry.get_str("mode"), Some("local"));
    assert_eq!(entry.get_str("origin"), Some("/o"));
    assert_eq!(entry.get_str("work_dir"), Some("/w"));
    assert_eq!(entry.get_str("goal"), Some("ship the login flow"));
    assert_eq!(
        entry.get("needs_attention"),
        Some(&json::Value::Bool(false)),
        "the always-present predicate is still always present"
    );
    assert_eq!(
        entry.get("degraded"),
        None,
        "SC-509b's omission rule for a healthy entry is unchanged"
    );
    assert!(entry.get("agents").is_some(), "the roster still renders");
}

// ---- criterion 23: completeness crosses classification and emission --------

#[test]
fn criterion_23_the_completeness_delta_survives_to_the_json_and_changes_nothing_else() {
    let scratch = Scratch::new("completeness");
    scratch.healthy("kept", "");
    let candidates = scratch.candidates();
    let losses = vec![
        FailedSource::CanonicalRoot(PathBuf::from("/home/x/.ae/sessions")),
        FailedSource::WorktreeRoot(PathBuf::from("/home/x/.ae/worktrees")),
    ];
    let answers = || Recorder::new().live(named("B"), &[("kept", Some("kept"))]);

    let complete = classify(
        Inventory {
            candidates: candidates.clone(),
            incomplete: Vec::new(),
        },
        &answers(),
    );
    let incomplete = classify(
        Inventory {
            candidates,
            incomplete: losses.clone(),
        },
        &answers(),
    );

    assert_eq!(
        sessions_of(&digest_of(&complete)),
        sessions_of(&digest_of(&incomplete)),
        "identities, statuses and degradation are identical across the pair"
    );
    assert_eq!(
        digest_of(&complete).get("inventory_complete"),
        Some(&json::Value::Bool(true))
    );
    assert_eq!(
        digest_of(&incomplete).get("inventory_complete"),
        Some(&json::Value::Bool(false))
    );
    assert_eq!(
        incomplete.incomplete, losses,
        "BOTH distinguishable loss facts crossed classification"
    );

    let empty_complete = classify(inventory(Vec::new()), &Recorder::new());
    let empty_incomplete = classify(
        Inventory {
            candidates: Vec::new(),
            incomplete: losses,
        },
        &Recorder::new(),
    );
    assert_eq!(
        digest_of(&empty_complete).get("inventory_complete"),
        Some(&json::Value::Bool(true))
    );
    assert_eq!(
        digest_of(&empty_incomplete).get("inventory_complete"),
        Some(&json::Value::Bool(false))
    );
}

#[test]
fn criterion_23_the_product_valid_arm_earns_the_delta_through_real_discovery() {
    let complete = Scratch::new("product-complete");
    assert!(
        fs::create_dir_all(complete.0.join("sessions")).is_ok(),
        "a readable, empty canonical root"
    );
    assert!(
        fs::create_dir_all(complete.0.join("worktrees")).is_ok(),
        "a readable, empty worktrees root"
    );
    let broken = Scratch::new("product-incomplete");
    assert!(
        fs::create_dir_all(broken.0.join("sessions")).is_ok(),
        "a readable canonical root"
    );
    assert!(
        fs::write(broken.0.join("worktrees"), "not a directory").is_ok(),
        "a hostile fixture"
    );
    assert!(
        fs::read_dir(broken.0.join("worktrees")).is_err(),
        "the enumeration operation itself must fail"
    );

    let complete_snapshot = classify(
        take(durable_records(&complete.roots()), None, &Recorder::new()),
        &Recorder::new(),
    );
    let broken_snapshot = classify(
        take(durable_records(&broken.roots()), None, &Recorder::new()),
        &Recorder::new(),
    );

    assert_eq!(
        sessions_of(&digest_of(&complete_snapshot)),
        sessions_of(&digest_of(&broken_snapshot)),
        "the candidate sets are equal — neither source had anything to lose"
    );
    assert_eq!(
        digest_of(&complete_snapshot).get("inventory_complete"),
        Some(&json::Value::Bool(true))
    );
    assert_eq!(
        digest_of(&broken_snapshot).get("inventory_complete"),
        Some(&json::Value::Bool(false)),
        "and the delta arrived through discovery rather than being handed in"
    );
}

// ---- criterion 22: phase 2 never widens phase-1 entitlement ----------------

#[test]
fn criterion_22_phase_2_contacts_only_servers_the_phase_1_facts_named() {
    let scratch = Scratch::new("entitlement");
    scratch.healthy("mine", "");
    for bait in ["tmux-1000", "sockets"] {
        assert!(fs::create_dir_all(scratch.0.join(bait)).is_ok(), "bait");
        assert!(
            fs::write(scratch.0.join(bait).join("default"), "socket").is_ok(),
            "bait socket"
        );
    }
    let candidates = scratch.candidates();
    let before = manifest(&scratch.0);

    let backend = Recorder::new()
        .live(named("B"), &[("mine", Some("mine"))])
        .live(ServerId::Ambient, &[("tempting", Some("tempting"))])
        .live(
            ServerId::Selected(Selector::Socket(
                scratch.0.join("tmux-1000").join("default"),
            )),
            &[("swept", Some("swept"))],
        );

    let snapshot = classify(inventory(candidates), &backend);
    let _ = digest_of(&snapshot);

    assert_eq!(
        backend.targets(),
        ["name:B"],
        "every contacted server came from the input's own typed facts"
    );
    assert_eq!(snapshot.sessions.len(), 1);
    assert_eq!(
        manifest(&scratch.0),
        before,
        "and no filesystem sweep looked for more servers"
    );
}

// ---- criterion 20: typed routing reaches the intended REAL server ----------

/// Run `tmux` with `args` through the harness's pinned process door, returning
/// `(succeeded, stdout)`.
///
/// The door is `parity::capture::raw::run` — the one place this test target may
/// name `std::process::Command`. Reusing it rather than opening a new one keeps
/// the crate's capability boundary exactly where `clippy.toml` pins it: the
/// LIBRARY still cannot spawn a process, which is what makes `src/tmux.rs` pure
/// argument derivation and interpretation rather than an adapter that shells
/// out.
fn run_tmux(args: &[String], scratch: &Path) -> (bool, String) {
    let mut invocation = Invocation::new("tmux");
    for arg in args {
        invocation = invocation.arg(arg);
    }
    let out = scratch.join("stdout");
    let err = scratch.join("stderr");
    let status = match raw::run(&invocation, scratch, &out, &err) {
        Ok(status) => status,
        Err(why) => panic!("tmux must be runnable for this proof: {why}"),
    };
    let succeeded = matches!(status.outcome(), ExitOutcome::Code(0));
    let stdout = fs::read_to_string(&out).unwrap_or_default();
    (succeeded, stdout)
}

/// Whether a real tmux is available to prove anything against.
fn tmux_present(scratch: &Path) -> bool {
    let out = scratch.join("probe-out");
    let err = scratch.join("probe-err");
    raw::run(&Invocation::new("tmux").arg("-V"), scratch, &out, &err)
        .is_ok_and(|status| matches!(status.outcome(), ExitOutcome::Code(0)))
}

#[test]
fn criterion_20_typed_name_and_socket_routing_reach_two_different_real_servers() {
    // Two ISOLATED tmux servers, addressed by the product's own derived
    // arguments. A mock that received the right argv would prove the mapping;
    // this proves the mapping ARRIVES — each server answers with its own
    // sessions and neither can answer for the other.
    //
    // Short socket path on purpose: `sun_path` is 104 bytes on macOS and the
    // usual temp dir eats most of it.
    let scratch = PathBuf::from(format!("/tmp/ae-p2-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    assert!(fs::create_dir_all(&scratch).is_ok(), "a short scratch dir");

    if !tmux_present(&scratch) {
        // STATED, never silent: this proof needs a real tmux, and an
        // environment without one has not run it. The unit half in
        // `src/tmux.rs` still pins the argument derivation.
        let _ = fs::remove_dir_all(&scratch);
        panic!(
            "tmux is not runnable here, so criterion 20's real-server half cannot be proven; \
             install tmux or run this suite where one exists"
        );
    }

    let socket_path = scratch.join("b.sock");
    let by_name = ServerId::Selected(Selector::Name(format!("ae-p2-{}", std::process::id())));
    let by_socket = ServerId::Selected(Selector::Socket(socket_path.clone()));

    // Create one session on each server, with DIFFERENT names, so an answer
    // from the wrong server is unmistakable.
    for (server, session) in [(&by_name, "on-the-named"), (&by_socket, "on-the-socket")] {
        let mut create = ae::tmux::server_args(server);
        create.extend(["new-session", "-d", "-s", session].map(ToOwned::to_owned));
        let (created, _) = run_tmux(&create, &scratch);
        assert!(created, "creating {session} must succeed");
    }

    // The product derives the argv; the door runs it; the product interprets it.
    let (named_ok, named_out) = run_tmux(&ae::tmux::list_sessions_args(&by_name), &scratch);
    let (socket_ok, socket_out) = run_tmux(&ae::tmux::list_sessions_args(&by_socket), &scratch);
    let named_sessions = ae::tmux::interpret_sessions(named_ok, &named_out);
    let socket_sessions = ae::tmux::interpret_sessions(socket_ok, &socket_out);

    // A failure arm on the same primitive: a socket that is not a server.
    let absent = ServerId::Selected(Selector::Socket(scratch.join("nothing-here.sock")));
    let (absent_ok, absent_out) = run_tmux(&ae::tmux::list_sessions_args(&absent), &scratch);
    let absent_sessions = ae::tmux::interpret_sessions(absent_ok, &absent_out);

    // Tear down before asserting, so a failure cannot leave servers behind.
    for server in [&by_name, &by_socket] {
        let mut kill = ae::tmux::server_args(server);
        kill.push("kill-server".to_owned());
        let _ = run_tmux(&kill, &scratch);
    }
    let _ = fs::remove_dir_all(&scratch);

    assert_eq!(
        named_sessions,
        Ok(vec!["on-the-named".to_owned()]),
        "-L routing reached the named server and nothing else"
    );
    assert_eq!(
        socket_sessions,
        Ok(vec!["on-the-socket".to_owned()]),
        "-S routing reached the socket server and nothing else"
    );
    assert_eq!(
        absent_sessions,
        Err(QueryFailed),
        "and a server that is not there is a FAILED query, not an empty success"
    );
}

#[test]
fn criterion_20_a_real_ownership_marker_round_trips_through_the_product_reader() {
    let scratch = PathBuf::from(format!("/tmp/ae-p2m-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    assert!(fs::create_dir_all(&scratch).is_ok(), "a short scratch dir");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the marker round trip cannot be proven");
    }

    let server = ServerId::Selected(Selector::Socket(scratch.join("m.sock")));
    let mut create = ae::tmux::server_args(&server);
    create.extend(
        [
            "new-session",
            "-d",
            "-s",
            "marked",
            "-e",
            "AE_SESSION=marked",
        ]
        .map(ToOwned::to_owned),
    );
    let (created, _) = run_tmux(&create, &scratch);
    let mut plain = ae::tmux::server_args(&server);
    plain.extend(["new-session", "-d", "-s", "unmarked"].map(ToOwned::to_owned));
    let (plain_made, _) = run_tmux(&plain, &scratch);

    let (marked_ok, marked_out) = run_tmux(&ae::tmux::marker_args(&server, "marked"), &scratch);
    let (unmarked_ok, unmarked_out) =
        run_tmux(&ae::tmux::marker_args(&server, "unmarked"), &scratch);
    let marked = ae::tmux::interpret_marker(marked_ok, &marked_out);
    let unmarked = ae::tmux::interpret_marker(unmarked_ok, &unmarked_out);

    let mut kill = ae::tmux::server_args(&server);
    kill.push("kill-server".to_owned());
    let _ = run_tmux(&kill, &scratch);
    let _ = fs::remove_dir_all(&scratch);

    assert!(created && plain_made, "both sessions must be created");
    assert_eq!(
        marked,
        Some("marked".to_owned()),
        "a real AE_SESSION reaches the product reader intact"
    );
    assert_eq!(
        unmarked, None,
        "and a session without one reports no marker rather than an empty string"
    );
    assert!(
        ae::liveness::positively_owned("marked", marked.as_deref()),
        "which is what the ownership predicate consumes"
    );
    assert!(!ae::liveness::positively_owned(
        "unmarked",
        unmarked.as_deref()
    ));
}

// ---- criterion 14, asked of the whole crate rather than one file ----------

/// The non-test source of every module in `src/`, comments stripped.
fn product_source() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let Ok(entries) = fs::read_dir(&src) else {
        panic!("src/ must be readable");
    };
    let mut modules: Vec<(String, String)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| {
            let Ok(text) = fs::read_to_string(&path) else {
                panic!("{} must be readable", path.display());
            };
            let code: String = text
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            let module = code
                .split_once("#[cfg(test)]")
                .map_or(code.clone(), |(module, _)| module.to_owned());
            let name = path
                .file_name()
                .map(|leaf| leaf.to_string_lossy().into_owned())
                .unwrap_or_default();
            (name, module)
        })
        .collect();
    modules.sort();
    modules
}

/// Where a needle appears in product code, file by file, with counts.
fn sites(needle: &str) -> Vec<(String, usize)> {
    product_source()
        .into_iter()
        .map(|(name, code)| (name, code.matches(needle).count()))
        .filter(|(_, count)| *count > 0)
        .collect()
}

#[test]
fn criterion_14_the_named_read_functions_appear_only_where_they_should() {
    // A TRIPWIRE, AND ITS LIMIT IS PART OF THE TEST. This scans for three
    // NAMES. It cannot see a filesystem observation spelled some other way —
    // `Path::exists`, `metadata`, `File::open`, a helper that wraps any of them
    // — and an earlier version of this guard claimed "exactly one filesystem
    // call outside the tests" while enforcing only these three. That claim was
    // broader than its enforcement, and a second observation lived inside it
    // for a whole review cycle.
    //
    // What closes more is
    // `criterion_14_the_digest_does_not_change_when_the_world_does` below —
    // but it closes AXES, not a class, and the difference is the whole reason
    // this comment was rewritten twice. A differential discriminates exactly
    // over the facts it MOVES; anything constant in every arm is invisible to
    // it, and an ABSENT input is the most constant thing there is. That test
    // therefore plants and opposes both record sources — meta AND events — and
    // covers those facts only. A source neither test plants is closed by
    // neither: no capability boundary exists for the filesystem here, so
    // nothing says "class".
    let modules = product_source();
    assert!(
        modules.iter().any(|(name, _)| name == "liveness.rs")
            && modules.iter().any(|(name, _)| name == "listing.rs"),
        "the scan reached the phase-2 modules"
    );

    assert_eq!(
        sites(concat!("Meta", "::read(")),
        vec![("session.rs".to_owned(), 1)],
        "the meta is opened in exactly one place"
    );
    assert_eq!(
        sites(concat!("SessionRead", "::open(")),
        vec![("session.rs".to_owned(), 1)],
        "and so is the event stream"
    );
    assert_eq!(
        sites(concat!("RecordSnapshot", "::read(")),
        vec![("inventory.rs".to_owned(), 1), ("session.rs".to_owned(), 1),],
        "the one reader is called from discovery, and from the convenience \
         wrapper that exists for callers who genuinely want a fresh read"
    );

    let Some((_, listing)) = product_source()
        .into_iter()
        .find(|(name, _)| name == "listing.rs")
    else {
        panic!("listing.rs must be scanned");
    };
    // The projection moved into `Presentation::world` when phase 3 made the
    // boundary a production type; the obligation did not move with it.
    let Some((_, projection)) = listing.split_once("pub fn world(") else {
        panic!("the projection must be in listing.rs");
    };
    for read in [
        concat!("entry_", "for("),
        concat!("Record", "Snapshot::read"),
        concat!("fs", "::"),
    ] {
        assert!(
            !projection.contains(read),
            "the digest must be assembled from the snapshot, never from a second read: {read}"
        );
    }
    assert!(
        projection.contains(concat!("entry_", "from(")),
        "it derives from the carried snapshot"
    );
}

/// Plant both halves of the record, so the differential has two axes to move.
///
/// Returns the path of the candidate whose meta is ABSENT and will appear later.
fn plant_both_record_sources(scratch: &Scratch) -> PathBuf {
    scratch.healthy("alpha", "goal=the original goal\n");
    // Event-derived facts SC-509 emits: goal_set_epoch (SC-405f), the declared
    // agent state (SC-510c), and last_active_epoch (SC-017e).
    scratch.events(
        "alpha",
        &[
            event("2026-05-29T09:00:00Z", "cl:lead", "goal", ""),
            event(
                "2026-05-29T10:00:00Z",
                "cl:lead",
                "state",
                r#","ref":"working""#,
            ),
        ],
    );
    scratch.degraded_but_addressable("damaged");
    let repaired = scratch.no_meta("was-absent");
    scratch.no_selector("quiet");
    repaired
}

/// Change the world under both axes, after the inventory was taken.
///
/// A meta reread would see a changed goal and a record that became readable; an
/// EVENT reread would see a later goal event, a later declared state and a
/// newer activity stamp. Every one of those is opposed to what was planted.
fn oppose_both_record_sources(scratch: &Scratch, repaired: &Path) {
    assert!(
        fs::write(
            repaired.join("meta"),
            "mode=local\nagent.main=cl:lead\ngoal=a goal that appeared later\n",
        )
        .is_ok(),
        "the absent record becomes readable"
    );
    assert!(
        fs::write(
            scratch.0.join("sessions").join("alpha").join("meta"),
            "mode=copy\nagent.main=cl:other\ngoal=a different goal\n",
        )
        .is_ok(),
        "and an existing record changes"
    );
    scratch.events(
        "alpha",
        &[
            event("2026-05-29T09:00:00Z", "cl:lead", "goal", ""),
            event(
                "2026-05-29T10:00:00Z",
                "cl:lead",
                "state",
                r#","ref":"working""#,
            ),
            // Strictly LATER, and opposed on every event-derived field.
            event("2026-05-29T11:00:00Z", "cl:lead", "goal", ""),
            event(
                "2026-05-29T12:00:00Z",
                "cl:lead",
                "state",
                r#","ref":"blocked""#,
            ),
        ],
    );
}

#[test]
fn criterion_14_the_digest_does_not_change_when_the_world_does() {
    // THE DIFFERENTIAL, over BOTH record sources — and over the facts it
    // plants, which is the limit of what it can say.
    //
    // A differential test discriminates exactly over the axes it MOVES, and an
    // ABSENT input is the most constant thing there is. An earlier version of
    // this test varied only the meta and gave every fixture no event log at
    // all — so SC-519 mapped that log to the same quiet stream in all three
    // arms, and an implementation rereading EVENTS could have passed every byte
    // comparison while being wrong. Deleting a source that was never there is
    // the weakest possible evidence about it.
    //
    // So both halves of the record are PLANTED, both are OPPOSED after
    // inventory, and every planted fact is asserted to have landed first.
    let scratch = Scratch::new("world-changes");
    let repaired = plant_both_record_sources(&scratch);
    let taken = take(durable_records(&scratch.roots()), None, &Recorder::new());

    let backend = || Recorder::new().live(named("B"), &[("alpha", Some("alpha"))]);
    let render_now = |inventory: Inventory| {
        render(
            &args(&["--all", "--json"]),
            &world_of(
                &classify(inventory, &backend()),
                NOW,
                DEFAULT_UNANSWERED_SECS,
            ),
        )
    };
    let before = render_now(taken.clone());

    // BOTH PRECONDITIONS, asserted before anything is concluded: each planted
    // fact must be VISIBLE in the first document, or the arm that is supposed
    // to discriminate carries nothing.
    // Derived from the planted stamps rather than hand-computed: a wrong
    // constant here would weaken the precondition into a tautology.
    let stamp = |iso: &str| match Timestamp::parse(iso) {
        Some(parsed) => parsed.epoch(),
        None => panic!("{iso} must parse"),
    };
    let planted_goal = stamp("2026-05-29T09:00:00Z");
    let planted_activity = stamp("2026-05-29T10:00:00Z");
    for planted in [
        r#""goal":"the original goal""#.to_owned(),
        format!(r#""goal_set_epoch":{planted_goal}"#),
        format!(r#""last_active_epoch":{planted_activity}"#),
        r#""state":"working""#.to_owned(),
    ] {
        assert!(
            before.contains(&planted),
            "the fixture must actually emit {planted}, or its axis proves nothing:\n{before}"
        );
    }

    oppose_both_record_sources(&scratch, &repaired);
    let after_growth = render_now(taken.clone());

    // The world DISAPPEARS. A read that FAILS would notice that.
    assert!(
        fs::remove_dir_all(scratch.0.join("sessions")).is_ok(),
        "and then it is gone"
    );
    let after_removal = render_now(taken);

    assert_eq!(
        before, after_growth,
        "the digest reports the snapshot, not the filesystem as it is now"
    );
    assert_eq!(before, after_removal, "in both directions, byte for byte");
    // WHICH ARM DOES THE WORK, because it is not the obvious one: a reread that
    // fails on a deleted tree can simply keep the carried value and pass the
    // removal arm. GROWTH is what catches a reread that SUCCEEDS and disagrees.
    // Deletion alone would have missed the very defect this test now proves
    // against.
    // Named explicitly, so a future reader can see which axes this closes.
    assert!(
        after_growth.contains(r#""state":"working""#)
            && !after_growth.contains(r#""state":"blocked""#),
        "the LATER declared state never reached the digest"
    );
    assert!(
        after_growth.contains(&format!(r#""goal_set_epoch":{planted_goal}"#))
            && !after_growth.contains(&format!(
                r#""goal_set_epoch":{}"#,
                stamp("2026-05-29T11:00:00Z")
            )),
        "nor the later goal event"
    );
    assert!(
        !after_growth.contains(&format!(
            r#""last_active_epoch":{}"#,
            stamp("2026-05-29T12:00:00Z")
        )),
        "nor the newer activity stamp"
    );
}

#[test]
fn criterion_14_a_record_that_changes_after_discovery_cannot_change_the_digest() {
    // The behavioural half of the same obligation, and the concrete failure the
    // rework fixes: a record unreadable at discovery must not become readable
    // before emission and REPAIR its own loss fact.
    let scratch = Scratch::new("second-observation");
    let damaged = scratch.no_meta("was-broken");
    let taken = take(durable_records(&scratch.roots()), None, &Recorder::new());

    // The world changes between the two boundaries. A second read would see it.
    assert!(
        fs::write(damaged.join("meta"), "mode=local\nagent.main=cl:lead\n").is_ok(),
        "the record becomes readable after discovery"
    );

    let snapshot = classify(taken, &Recorder::new());
    let emitted = sessions_of(&digest_of(&snapshot));

    assert_eq!(
        emitted,
        vec![("was-broken".to_owned(), "unknown".to_owned(), true)],
        "the digest reports what discovery SAW: still degraded, still unknown"
    );
    let document = digest_of(&snapshot);
    let Some(json::Value::Arr(entries)) = document.get("sessions") else {
        panic!("sessions must be an array");
    };
    assert_eq!(
        entries[0].get_str("mode"),
        None,
        "and it did not pick up a field that only exists in the later bytes"
    );
}
