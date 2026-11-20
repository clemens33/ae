//! Frozen R4 observation contract, against base 43c76c5d.
//! Parent supplies `FleetDiscovery` and `world_read`; register beside their owner.

#![allow(
    clippy::disallowed_methods,
    reason = "frozen acceptance fixtures own and inspect disposable record directories"
)]

use super::{FleetDiscovery, world_read};
use crate::inventory::{DiscoveredSession, Discovery, QueryFailed, RecordedOn, ServerId};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    static TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct Changing {
    calls: RefCell<Vec<ServerId>>,
    fail_first: bool,
}

impl Discovery for Changing {
    fn enumerate(&self, server: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed> {
        let repeated = self.calls.borrow().contains(server);
        self.calls.borrow_mut().push(server.clone());
        if self.fail_first && !repeated {
            Err(QueryFailed)
        } else if repeated {
            Ok(vec![DiscoveredSession {
                name: "contradiction".into(),
                marker: Some("1".into()),
            }])
        } else {
            Ok(Vec::new())
        }
    }
}

#[test]
fn r4_discovery_caches_success_and_failure_before_a_contradictory_second_answer() {
    // Mutation: delete settled lookup or cache only successful answers.
    let server = ServerId::Selected(crate::meta::Selector::Name("r4-private".into()));
    for fail_first in [false, true] {
        let inner = Changing {
            calls: RefCell::default(),
            fail_first,
        };
        let recorded = RecordedOn::default();
        let discovery = FleetDiscovery {
            inner: &inner,
            default_is_optional: false,
            recorded: &recorded,
            boot: None,
            now: 0,
            settled: RefCell::default(),
        };
        let expected = if fail_first {
            Err(QueryFailed)
        } else {
            Ok(Vec::new())
        };
        assert_eq!(discovery.enumerate(&server), expected);
        assert_eq!(discovery.enumerate(&server), expected);
        assert_eq!(
            inner.calls.borrow().as_slice(),
            std::slice::from_ref(&server)
        );
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // world_read has an opportunistic -L ae read. Require the lane's
        // existing private TMUX_TMPDIR before that read can occur.
        let tmux = std::env::var_os("TMUX_TMPDIR")
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .expect("run R4 world pins through just _tmux-isolated test");
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = tmux.join(format!("r4-unit-{}-{n}", std::process::id()));
        assert!(std::fs::create_dir_all(&root).is_ok());
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        crate::set_after_classify_hook(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn oppose(root: &Path) {
    TRACE.with(|trace| trace.borrow_mut().push("classified".into()));
    let path = root.join("sessions/alpha");
    assert!(
        std::fs::write(
            path.join("meta"),
            "mode=copy\nseat.main=r4-opposed-seat\nprofile.main=idle\ngoal=opposed\n"
        )
        .is_ok()
    );
    assert!(std::fs::write(path.join("events.jsonl"), "{\"ts\":\"2026-05-29T12:00:00Z\",\"actor\":\"r4-captured-seat\",\"action\":\"state\",\"ref\":\"blocked\"}\n").is_ok());
}

fn assert_captured_alpha(out: &str) {
    use crate::json::{self, Value};
    let document = json::parse(out).expect("list JSON");
    let Some(Value::Arr(sessions)) = document.get("sessions") else {
        panic!("sessions array");
    };
    let alpha = sessions
        .iter()
        .find(|row| row.get_str("name") == Some("alpha"))
        .expect("alpha from canonical source");
    let Some(Value::Arr(agents)) = alpha.get("agents") else {
        panic!("captured alpha roster");
    };
    assert_eq!(agents.len(), 1, "one captured seat: {out}");
    assert_eq!(
        agents[0].get_str("name"),
        Some("r4-captured-seat"),
        "captured roster survives: {out}"
    );
    assert_eq!(
        agents[0].get_str("state"),
        Some("working"),
        "captured journal survives: {out}"
    );
}

#[test]
fn r4_world_reads_each_durable_source_once_then_keeps_captured_facts() {
    // Mutations: scan twice; reread after classification; replace a carried
    // snapshot with RecordSnapshot::read before projection.
    TRACE.with(|trace| trace.borrow_mut().clear());
    let scratch = Scratch::new();
    let alpha = scratch.0.join("sessions/alpha");
    let nested = scratch.0.join("worktrees/project/.ae/beta");
    for path in [&alpha, &nested] {
        assert!(std::fs::create_dir_all(path).is_ok());
        assert!(
            std::fs::write(
                path.join("meta"),
                "mode=local\nseat.main=r4-captured-seat\nprofile.main=idle\ngoal=captured\n"
            )
            .is_ok()
        );
        assert!(std::fs::write(path.join("events.jsonl"), "{\"ts\":\"2026-05-29T09:00:00Z\",\"actor\":\"r4-captured-seat\",\"action\":\"state\",\"ref\":\"working\"}\n").is_ok());
    }
    crate::set_after_classify_hook(Some(oppose));
    let read = world_read(&scratch.0, &mut |path| {
        let label = path
            .strip_prefix(&scratch.0)
            .expect("owned record")
            .to_string_lossy();
        TRACE.with(|trace| trace.borrow_mut().push(format!("record:{label}")));
        crate::session::RecordSnapshot::read(path)
    });
    assert_eq!(
        read.snapshot
            .sessions
            .iter()
            .filter(|row| row.candidate.durable.is_some())
            .count(),
        2,
        "both durable layouts reached classification"
    );
    TRACE.with(|trace| {
        assert_eq!(
            *trace.borrow(),
            [
                "record:sessions/alpha",
                "record:worktrees/project/.ae/beta",
                "classified"
            ]
        );
    });
    let mut out = Vec::new();
    let mut err = Vec::new();
    assert_eq!(
        crate::run_with(
            &["list".into(), "--all".into(), "--json".into()],
            Some(&read.world),
            &mut out,
            &mut err
        )
        .expect("render"),
        0
    );
    let out = String::from_utf8(out).expect("json utf8");
    assert_captured_alpha(&out);
    assert_eq!(
        out.matches("\"goal\":\"captured\"").count(),
        2,
        "both captured goals survive: {out}"
    );
    assert!(
        !out.contains("opposed"),
        "later disk bytes cannot replace captured facts: {out}"
    );
    assert!(
        std::fs::read_to_string(alpha.join("meta"))
            .expect("opposed meta")
            .contains("opposed"),
        "non-vacuity: hook changed the disk"
    );
}

#[test]
fn r4_world_returns_the_socket_cache_it_used_without_a_second_query() {
    // Mutation: return a fresh SocketPaths at the WorldRead tail. Observe
    // actual transport calls through its existing observer; replace nothing.
    let scratch = Scratch::new();
    let (mut read, first) = crate::transport::spawns_during(|| {
        world_read(&scratch.0, &mut crate::session::RecordSnapshot::read)
    });
    assert_eq!(
        first
            .iter()
            .filter(|argv| argv.iter().any(|word| word == "#{socket_path}"))
            .count(),
        1,
        "world read performed its one default-server socket query: {first:?}"
    );
    let own = crate::instance::own_server();
    let (_, later) = crate::transport::spawns_during(|| read.sockets.proven_same(&own, &own));
    assert!(
        later.is_empty(),
        "returned cache must retain the earlier socket answer, failure included: {later:?}"
    );
}
