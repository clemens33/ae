//! Phase 2 end to end: phase-1 facts in, classified liveness and a
//! schema-version-2 digest out.
//!
//! Gate: `docs/migration/p1-phase2-gate.md`, blob
//! `29db943aa85319534301332052105ba16df03b4d`. Each test names the criterion it
//! answers. The unit-level halves live beside their code in `src/liveness.rs`
//! and `src/tmux.rs`; what is here is what needs a real filesystem, a real tmux,
//! or the whole pipeline at once.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use ae::digest::Status;
use ae::inventory::{
    Candidate, DiscoveredSession, Discovery, DurableRecord, FailedSource, Inventory, Layout,
    LiveSighting, MetaRead, QueryFailed, Roots, ServerId, durable_records, take,
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

    /// A session directory with a readable meta naming one agent — the
    /// not-degraded shape.
    fn healthy(&self, name: &str, meta_extra: &str) -> PathBuf {
        let dir = self.0.join("sessions").join(name);
        let written = fs::create_dir_all(&dir).and_then(|()| {
            fs::write(
                dir.join("meta"),
                format!("mode=local\nagent.main=cl:lead\n{meta_extra}"),
            )
        });
        assert!(written.is_ok(), "a session fixture must be writable");
        dir
    }

    /// A session directory whose meta is absent — SC-405i record loss.
    fn no_meta(&self, name: &str) -> PathBuf {
        let dir = self.0.join("sessions").join(name);
        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "a session fixture must be creatable"
        );
        dir
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

fn record_at(path: &Path, server: ServerSelector, meta_read: MetaRead) -> DurableRecord {
    DurableRecord {
        path: path.to_path_buf(),
        name: match path.file_name() {
            Some(leaf) => leaf.to_string_lossy().into_owned(),
            None => panic!("a state directory always has a last component"),
        },
        layout: Layout::Canonical,
        server,
        meta_read,
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

// ---- criterion 1 and 14: the boundary, and what does not cross it ----------

#[test]
fn criterion_1_the_complete_inventory_is_what_classification_receives() {
    let scratch = Scratch::new("boundary");
    scratch.healthy("alpha", "");
    scratch.healthy("beta", "");
    let backend = Recorder::new();

    // `inventory complete`: the value, captured whole.
    let taken = take(durable_records(&scratch.roots()), None, &backend);
    let at_inventory: Vec<String> = taken
        .candidates
        .iter()
        .map(|candidate| candidate.name.clone())
        .collect();
    assert!(
        backend.records().is_empty(),
        "nothing classified anything yet: the backend has not been asked"
    );

    // `classify enter`: the same value, and the classifier's input IS it.
    let snapshot = classify(taken, &backend);
    let at_classify: Vec<String> = snapshot
        .sessions
        .iter()
        .map(|classified| classified.candidate.name.clone())
        .collect();

    assert_eq!(at_inventory, at_classify, "the two semantic sets are equal");
    assert_eq!(at_classify.len(), 2, "and nothing was materialized inside");
}

#[test]
fn criterion_14_classification_survives_the_filesystem_disappearing_under_it() {
    // The strongest form of "phase 2 does not rediscover": take the inventory,
    // then DELETE the roots. A classifier that re-read anything would change
    // its answer; this one cannot, because it holds every fact it needs.
    let scratch = Scratch::new("no-reread");
    scratch.healthy("alpha", "");
    let mut record = record_at(
        &scratch.0.join("sessions").join("alpha"),
        positive("B"),
        MetaRead::Parsed,
    );
    record.name = "alpha".to_owned();
    let taken = inventory(vec![Candidate::durable(record)]);

    fs::remove_dir_all(scratch.0.join("sessions")).expect("the roots go away");

    let backend = Recorder::new().live(named("B"), &[("alpha", Some("alpha"))]);
    let snapshot = classify(taken, &backend);

    assert_eq!(snapshot.sessions[0].status, Status::Running);
    assert_eq!(
        backend.targets(),
        ["name:B"],
        "one backend query, no rescan"
    );
}

#[test]
fn criterion_9_a_tmux_only_candidate_leaves_the_filesystem_untouched() {
    // SC-017k reuse, plus the "no durable record is fabricated" half: capture
    // the tree before and after, and require it byte-identical.
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

    assert_eq!(snapshot.sessions[0].status, Status::Running);
    assert!(
        backend.records().is_empty(),
        "A was never re-queried, so its unavailability could not downgrade anything"
    );
    assert!(snapshot.sessions[0].candidate.durable.is_none());
    assert_eq!(
        manifest(&scratch.0),
        before,
        "transient discovery evidence was not persisted as durable state"
    );
}

/// Every path under `root`, with its kind and content length — enough to see a
/// write that a directory listing alone would miss.
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

#[test]
fn criterion_13_unknown_and_degraded_are_independent_at_both_boundaries() {
    let scratch = Scratch::new("orthogonal");
    // Degradation comes from a RECORD fact, never from breaking the query:
    // a meta with no roster is SC-405k/SC-405i loss; a meta with one is not.
    let running_clean = scratch.healthy("running-clean", "");
    let running_damaged = scratch.no_meta("running-damaged");
    let unknown_clean = scratch.healthy("unknown-clean", "");
    let unknown_damaged = scratch.no_meta("unknown-damaged");
    let stopped_clean = scratch.healthy("stopped-clean", "");
    let stopped_damaged = scratch.no_meta("stopped-damaged");

    let candidates = vec![
        Candidate::durable(record_at(&running_clean, positive("B"), MetaRead::Parsed)),
        Candidate::durable(record_at(&running_damaged, positive("B"), MetaRead::Absent)),
        // The unknown pair holds selector state at SC-405l `missing`, and moves
        // ONLY the record-read fact.
        Candidate::durable(record_at(
            &unknown_clean,
            ServerSelector::Missing,
            MetaRead::Parsed,
        )),
        Candidate::durable(record_at(
            &unknown_damaged,
            ServerSelector::Missing,
            MetaRead::Absent,
        )),
        Candidate::durable(record_at(&stopped_clean, positive("B"), MetaRead::Parsed)),
        Candidate::durable(record_at(&stopped_damaged, positive("B"), MetaRead::Absent)),
    ];
    let backend = Recorder::new().live(
        named("B"),
        &[
            ("running-clean", Some("running-clean")),
            ("running-damaged", Some("running-damaged")),
        ],
    );

    let snapshot = classify(inventory(candidates), &backend);

    // At the classifier boundary.
    let classified: Vec<(String, Status)> = snapshot
        .sessions
        .iter()
        .map(|entry| (entry.candidate.name.clone(), entry.status))
        .collect();
    assert_eq!(
        classified,
        vec![
            ("running-clean".to_owned(), Status::Running),
            ("running-damaged".to_owned(), Status::Running),
            ("unknown-clean".to_owned(), Status::Unknown),
            ("unknown-damaged".to_owned(), Status::Unknown),
            ("stopped-clean".to_owned(), Status::Stopped),
            ("stopped-damaged".to_owned(), Status::Stopped),
        ],
        "flipping degradation alone never moved a status"
    );

    // ...and in the emitted schema-version-2 digest.
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
        "every combination survives serialization: unknown does not set degraded, \
         degraded does not force unknown, and the serializer treats neither specially"
    );
}

// ---- criterion 16: every successor digest, every shape --------------------

#[test]
fn criterion_16_every_successor_digest_is_version_2_with_a_closed_status_domain() {
    let scratch = Scratch::new("matrix");
    let running = scratch.healthy("r", "");
    let stopped = scratch.healthy("s", "");
    let unknown = scratch.healthy("u", "");

    let shapes: Vec<(&str, Vec<Candidate>)> = vec![
        ("empty", Vec::new()),
        (
            "running only",
            vec![Candidate::durable(record_at(
                &running,
                positive("B"),
                MetaRead::Parsed,
            ))],
        ),
        (
            "stopped only",
            vec![Candidate::durable(record_at(
                &stopped,
                positive("B"),
                MetaRead::Parsed,
            ))],
        ),
        (
            "unknown only",
            vec![Candidate::durable(record_at(
                &unknown,
                ServerSelector::Ambiguous,
                MetaRead::Parsed,
            ))],
        ),
        (
            "mixed",
            vec![
                Candidate::durable(record_at(&running, positive("B"), MetaRead::Parsed)),
                Candidate::durable(record_at(&stopped, positive("B"), MetaRead::Parsed)),
                Candidate::durable(record_at(
                    &unknown,
                    ServerSelector::Ambiguous,
                    MetaRead::Parsed,
                )),
            ],
        ),
    ];

    for (shape, candidates) in shapes {
        for complete in [true, false] {
            let backend = Recorder::new().live(named("B"), &[("r", Some("r"))]);
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
            let world = world_of(&snapshot, NOW, DEFAULT_UNANSWERED_SECS);
            let rendered = render(&args(&["--all", "--json"]), &world);
            let document = json::parse(rendered.trim_end())
                .unwrap_or_else(|_| panic!("{shape}/{complete}: valid JSON"));

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
    // The successor has no version-selectable serializer — there is one schema
    // and it is 2 — so the implication closes the way the criterion allows: every
    // emitted document is searched, and none carries the forbidden pair.
    let scratch = Scratch::new("v1-never-unknown");
    let unknown = scratch.healthy("u", "");
    let snapshot = classify(
        inventory(vec![Candidate::durable(record_at(
            &unknown,
            ServerSelector::Ambiguous,
            MetaRead::Parsed,
        ))]),
        &Recorder::new(),
    );
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
    let running = scratch.healthy("r", "");
    let stopped = scratch.healthy("s", "");
    let unknown = scratch.healthy("u", "");
    let backend = Recorder::new().live(named("B"), &[("r", Some("r"))]);
    let snapshot = classify(
        inventory(vec![
            Candidate::durable(record_at(&running, positive("B"), MetaRead::Parsed)),
            Candidate::durable(record_at(&stopped, positive("B"), MetaRead::Parsed)),
            Candidate::durable(record_at(
                &unknown,
                ServerSelector::Ambiguous,
                MetaRead::Parsed,
            )),
        ]),
        &backend,
    );

    let classified: Vec<(String, Status)> = snapshot
        .sessions
        .iter()
        .map(|entry| (entry.candidate.name.clone(), entry.status))
        .collect();

    // Every view over the SAME classification. Which rows a filter shows is
    // SC-017m/n's business and is not asserted here; what must not move is the
    // status attached to a row that IS shown.
    for flags in [
        vec!["--json"],
        vec!["--all", "--json"],
        vec!["--stopped", "--json"],
        vec!["--needs-attn", "--all", "--json"],
    ] {
        let world = world_of(&snapshot, NOW, DEFAULT_UNANSWERED_SECS);
        let document = json::parse(render(&args(&flags), &world).trim_end()).expect("one document");
        for (name, status, _) in sessions_of(&document) {
            let expected = classified
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, status)| *status)
                .expect("a classified session");
            assert_eq!(
                status,
                expected.as_str(),
                "{flags:?}: {name} was rendered with a different status than it was classified"
            );
        }
    }

    // The human route reads the same answer.
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
    let dir = scratch.healthy(
        "my-feature",
        "origin=/o\nwork_dir=/w\ngoal=ship the login flow\n",
    );
    let snapshot = classify(
        inventory(vec![Candidate::durable(record_at(
            &dir,
            positive("B"),
            MetaRead::Parsed,
        ))]),
        &Recorder::new().live(named("B"), &[("my-feature", Some("my-feature"))]),
    );
    let document = digest_of(&snapshot);

    assert_eq!(document.get("schema_version"), Some(&json::Value::Num(2)));
    assert!(document.get("generated_at").is_some(), "SC-509 field kept");
    let Some(json::Value::Arr(entries)) = document.get("sessions") else {
        panic!("sessions must be an array");
    };
    let entry = &entries[0];
    // The documented SC-509 field set, still present and still meaning the same.
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
    let dir = scratch.healthy("kept", "");
    let candidates = vec![Candidate::durable(record_at(
        &dir,
        positive("B"),
        MetaRead::Parsed,
    ))];
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

    // Empty on both sides, so an incomplete-empty snapshot cannot pass as an
    // authoritative empty one.
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
    // The controlled pair above proves classifier preservation. This proves the
    // delta can ARISE: one readable-empty source (complete), one whose
    // enumeration demonstrably fails (incomplete), with the candidate set equal
    // because the failing source is the one that had nothing to contribute.
    let complete = Scratch::new("product-complete");
    fs::create_dir_all(complete.0.join("sessions")).expect("a readable, empty canonical root");
    fs::create_dir_all(complete.0.join("worktrees")).expect("a readable, empty worktrees root");
    let broken = Scratch::new("product-incomplete");
    fs::create_dir_all(broken.0.join("sessions")).expect("a readable canonical root");
    fs::write(broken.0.join("worktrees"), "not a directory").expect("a hostile fixture");
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
    let mine = scratch.healthy("mine", "");
    // Plausible bait on disk beside the roots.
    for bait in ["tmux-1000", "sockets"] {
        fs::create_dir_all(scratch.0.join(bait)).expect("bait");
        fs::write(scratch.0.join(bait).join("default"), "socket").expect("bait socket");
    }
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

    let snapshot = classify(
        inventory(vec![Candidate::durable(record_at(
            &mine,
            positive("B"),
            MetaRead::Parsed,
        ))]),
        &backend,
    );

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
