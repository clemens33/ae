//! One fleet read's owner: one record snapshot per durable source, one
//! cache, one assembly. [`world_read`] is the read; the rest serves it.

use crate::{
    digest, doors, inventory, listing, liveness, meta, read_gate, session, shape, time, tmux,
    transport,
};

/// The socket path each server answers with, asked once per server.
///
/// One read per DISTINCT server, however many sessions name it: a fleet on one
/// server is one question, not one per row.
pub(crate) struct SocketPaths {
    /// Who is asked. A parameter so the collision the cache exists to catch can
    /// be proven without two real servers.
    resolve: fn(&inventory::ServerId) -> Option<String>,
    seen: Vec<(inventory::ServerId, Option<String>)>,
}

impl SocketPaths {
    /// A cache that asks tmux.
    pub(crate) const fn live() -> Self {
        Self {
            resolve: transport::observe_socket_path,
            seen: Vec::new(),
        }
    }

    /// [`Self::live`] already knowing what the CALLER server answered its own
    /// batched read, so the picker's stopped scan does not ask the same server
    /// twice.
    pub(crate) fn for_caller(caller: &inventory::ServerId, socket: Option<String>) -> Self {
        Self {
            resolve: transport::observe_socket_path,
            seen: vec![(caller.clone(), socket)],
        }
    }

    /// A cache that asks `resolve`.
    #[cfg(test)]
    pub(crate) const fn asking(resolve: fn(&inventory::ServerId) -> Option<String>) -> Self {
        Self {
            resolve,
            seen: Vec::new(),
        }
    }

    /// [`Self::for_caller`] with `resolve` in tmux's place.
    #[cfg(test)]
    pub(crate) fn primed(
        resolve: fn(&inventory::ServerId) -> Option<String>,
        caller: &inventory::ServerId,
        socket: Option<String>,
    ) -> Self {
        Self {
            resolve,
            seen: vec![(caller.clone(), socket)],
        }
    }

    /// What `server` calls its own socket, or `None` when it did not answer.
    fn of(&mut self, server: &inventory::ServerId) -> Option<String> {
        if let Some((_, path)) = self.seen.iter().find(|(known, _)| known == server) {
            return path.clone();
        }
        let path = (self.resolve)(server);
        self.seen.push((server.clone(), path.clone()));
        path
    }

    /// Whether both server spellings answered with the same socket path.
    /// Unknown is never equal, even when the spellings themselves match.
    pub(crate) fn proven_same(
        &mut self,
        left: &inventory::ServerId,
        right: &inventory::ServerId,
    ) -> bool {
        let left = self.of(left);
        left.is_some() && self.of(right) == left
    }

    /// Reuse this fleet read's proof; an uncached app socket is already held
    /// from tmux's own marker. A cached failure always wins over that spelling.
    pub(crate) fn known_same(
        &self,
        recorded: &inventory::ServerId,
        app: &inventory::ServerId,
    ) -> bool {
        let cached = |server| self.seen.iter().find(|(known, _)| known == server);
        let Some((_, Some(path))) = cached(recorded) else {
            return false;
        };
        match cached(app) {
            Some((_, answer)) => answer.as_ref() == Some(path),
            None => matches!(app, inventory::ServerId::Selected(meta::Selector::Socket(socket))
                if socket.as_os_str().to_str() == Some(path.as_str())),
        }
    }

    /// Keep the first spelling of each server whose socket identity tmux can
    /// prove. Recorded pointers precede the default named entitlement, so a
    /// durable record retains its own spelling when both name one server.
    pub(crate) fn deduplicated(
        &mut self,
        servers: Vec<inventory::ServerId>,
    ) -> Vec<inventory::ServerId> {
        let mut paths: Vec<String> = Vec::new();
        let mut distinct = Vec::new();
        for server in servers {
            let path = self.of(&server);
            if path.as_ref().is_some_and(|path| paths.contains(path)) {
                continue;
            }
            if let Some(path) = path {
                paths.push(path);
            }
            distinct.push(server);
        }
        distinct
    }

    /// Whether two spellings were proven to name one socket while building a
    /// de-duplicated entitlement set.
    pub(crate) fn equivalent(
        &self,
        left: &inventory::ServerId,
        right: &inventory::ServerId,
    ) -> bool {
        if left == right {
            return true;
        }
        let known = |server: &inventory::ServerId| {
            self.seen
                .iter()
                .find(|(candidate, _)| candidate == server)
                .and_then(|(_, path)| path.as_deref())
        };
        known(left).is_some() && known(left) == known(right)
    }
}

/// Fleet discovery with one opportunistic source: ae's built-in server is
/// useful before any meta points at it, but its cold absence is not inventory
/// loss. Once a durable record names it, an unanswered query is loss again.
/// Warm launches record the socket spelling instead; that server's own failed
/// socket query still carries the loss while the unused name stays optional.
struct FleetDiscovery<'a, D> {
    inner: &'a D,
    default_is_optional: bool,
    /// Which sessions each recorded server holds, for the boot-time reading of
    /// a server whose socket went with the host.
    recorded: &'a inventory::RecordedOn,
    boot: Option<i64>,
    now: i64,
    /// What each server answered during this read, a failure included, so the
    /// inventory and the classification see ONE answer per server.
    settled: std::cell::RefCell<Vec<(inventory::ServerId, ServerAnswer)>>,
}

/// What one enumeration of a server answered.
type ServerAnswer = std::result::Result<Vec<inventory::DiscoveredSession>, inventory::QueryFailed>;

impl<D: inventory::Discovery> inventory::Discovery for FleetDiscovery<'_, D> {
    fn enumerate(&self, server: &inventory::ServerId) -> ServerAnswer {
        let known = self
            .settled
            .borrow()
            .iter()
            .find(|(asked, _)| asked == server)
            .map(|(_, answer)| answer.clone());
        if let Some(answer) = known {
            return answer;
        }
        let answer = self.ask(server);
        self.settled
            .borrow_mut()
            .push((server.clone(), answer.clone()));
        answer
    }
}

impl<D: inventory::Discovery> FleetDiscovery<'_, D> {
    /// The fresh question behind `enumerate`, asked at most once per server per
    /// read: its answer stands for both phases, and a failure is not retried
    /// into a different verdict.
    fn ask(&self, server: &inventory::ServerId) -> ServerAnswer {
        match self.inner.enumerate(server) {
            Err(_) if self.default_is_optional && server == &crate::instance::own_server() => {
                Ok(Vec::new())
            }
            // A REBOOT, and the listing's version of the resume's proof: this
            // server did not answer, its socket is not there at all, and every
            // session ae recorded on it was last live before the host booted.
            // None of them can be on it, so it enumerated EMPTY rather than
            // failing — the rows read `stopped` instead of `unknown` and the
            // listing stops calling itself incomplete over it. One gap in that
            // evidence, and it is a failed source again.
            Err(failed)
                if self.recorded.all_predate(server, self.boot, self.now)
                    && transport::server_socket_missing(server) =>
            {
                let _ = failed;
                Ok(Vec::new())
            }
            answer => answer,
        }
    }
}

/// The classified snapshot AND the world `ae list` shows right now — the real
/// route, returned in both halves so it can be observed from outside.
#[must_use]
pub fn current_world(root: &std::path::Path) -> (liveness::Snapshot, listing::World) {
    let read = world_read(root, &mut session::RecordSnapshot::read);
    (read.snapshot, read.world)
}

/// One [`current_world`] read, and what tmux listed of each Running record's
/// panes during it.
pub(crate) struct WorldRead {
    pub(crate) snapshot: liveness::Snapshot,
    pub(crate) world: listing::World,
    pub(crate) panes: ListedPanes,
    pub(crate) pids: SeatPids,
    pub(crate) sockets: SocketPaths,
}

/// What one listed pane shows of a seat: its root pid and foreground command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeatPane {
    pub(crate) pid: Option<u32>,
    pub(crate) command: Option<String>,
}

/// Each Running record's listed pane per slot by its state directory; `None`
/// where more than one pane carries the slot.
pub(crate) type SeatPids = std::collections::BTreeMap<
    std::path::PathBuf,
    std::collections::BTreeMap<String, Option<SeatPane>>,
>;

/// Each Running record's panes by its state directory: `None` when the read
/// listed none (no recorded server, no meta, or tmux did not answer).
pub(crate) type ListedPanes =
    std::collections::BTreeMap<std::path::PathBuf, Option<Vec<session::AgentRuntime>>>;

/// [`current_world`] with each durable record read by `read`.
pub(crate) fn world_read(
    root: &std::path::Path,
    read: &mut dyn FnMut(&std::path::Path) -> session::RecordSnapshot,
) -> WorldRead {
    read_gate("@world");
    let scan = inventory::durable_records_with(&inventory::Roots::under(root), read);
    // No caller server: fleet discovery spans recorded destinations plus ae's
    // own default server. A recorded socket may be another spelling of `-L
    // ae`; tmux's own socket answer is the only proof that lets us query it
    // once without losing the durable record's preferred spelling.
    let entitled = inventory::entitled_servers(None, &scan.records);
    let mut sockets = SocketPaths::live();
    let entitled = sockets.deduplicated(entitled);
    let default_recorded = scan.records.iter().any(|record| {
        record.server.entitles() == Some(&meta::Selector::Name(crate::instance::own_server_name()))
    });
    let tmux = transport::Tmux;
    let recorded = inventory::RecordedOn::of(&scan.records);
    let discovery = FleetDiscovery {
        inner: &tmux,
        default_is_optional: !default_recorded,
        recorded: &recorded,
        boot: doors::boot_time(shape::current()),
        now: time::Timestamp::now().epoch(),
        settled: std::cell::RefCell::default(),
    };
    let taken = inventory::take_entitled(scan, entitled, &discovery, |left, right| {
        sockets.equivalent(left, right)
    });
    // The SAME discovery, and so the same ANSWER per server, for the
    // classification: a server the boot-time proof read as empty must read as
    // empty in both phases, or the rows would go `unknown` under a listing that
    // no longer says anything is missing.
    let snapshot = liveness::classify(taken, &discovery);
    // Criterion 3 only: the opposed disk must change HERE, on this function's
    // path, not after it returns.
    #[cfg(debug_assertions)]
    AFTER_CLASSIFY.with(|slot| {
        if let Some(hook) = slot.get() {
            hook(root);
        }
    });
    let (runtimes, panes, pids) = observed_runtimes(&snapshot);
    let world = listing::Presentation::enter(&snapshot).world_with(
        time::Timestamp::now(),
        session::DEFAULT_UNANSWERED_SECS,
        &runtimes,
    );
    WorldRead {
        snapshot,
        world,
        panes,
        pids,
        sockets,
    }
}

/// What tmux says RIGHT NOW about every classified candidate, in snapshot
/// order, and each Running record's panes it listed. Each server holding a
/// Running record is asked ONCE (`list-panes -a`), the branch riding on that
/// same format.
fn observed_runtimes(
    snapshot: &liveness::Snapshot,
) -> (Vec<session::SessionRuntime>, ListedPanes, SeatPids) {
    let mut panes = ListedPanes::new();
    let mut pids = SeatPids::new();
    let mut asked: Vec<(inventory::ServerId, Option<tmux::ServerPanes>)> = Vec::new();
    let runtimes = snapshot
        .sessions
        .iter()
        .map(|classified| {
            let mut runtime = session::SessionRuntime::new(classified.status);
            if classified.status != digest::Status::Running {
                return runtime;
            }
            let Some(record) = classified.candidate.durable.as_ref() else {
                return runtime;
            };
            let listed = panes.entry(record.path.clone()).or_insert(None);
            let Some(selector) = record.server.entitles() else {
                return runtime;
            };
            let server = inventory::ServerId::Selected(selector.clone());
            let at = asked
                .iter()
                .position(|(known, _)| *known == server)
                .unwrap_or_else(|| {
                    asked.push((server.clone(), transport::observe_all_panes(&server)));
                    asked.len() - 1
                });
            let session = asked[at]
                .1
                .as_ref()
                .and_then(|answer| answer.session(&record.name));
            runtime.branch = session.and_then(|rows| rows.branch.clone());
            if let Some(rows) = session {
                let seats = pids.entry(record.path.clone()).or_default();
                for pane in &rows.panes {
                    if let Some(slot) = &pane.slot {
                        seats
                            .entry(slot.clone())
                            .and_modify(|held| *held = None)
                            .or_insert_with(|| {
                                Some(SeatPane {
                                    pid: pane.pid,
                                    command: pane.command.clone(),
                                })
                            });
                    }
                }
            }
            *listed = record
                .snapshot
                .meta
                .as_ref()
                .zip(session)
                .map(|(meta, rows)| agents_from_panes(&rows.panes, meta));
            if let Some(agents) = listed {
                runtime.agents.clone_from(agents);
            }
            runtime
        })
        .collect();
    (runtimes, panes, pids)
}

/// What tmux says RIGHT NOW of each roster seat of session `name` on `server`;
/// `None` when it did not list the session's panes.
pub(crate) fn observed_agents(
    server: &inventory::ServerId,
    name: &str,
    meta: &meta::Meta,
) -> Option<Vec<session::AgentRuntime>> {
    let panes = transport::observe_panes(server, name)?;
    Some(agents_from_panes(&panes, meta))
}

/// The roster seats of `meta` as `panes` show them.
fn agents_from_panes(
    panes: &[tmux::ObservedPane],
    meta: &meta::Meta,
) -> Vec<session::AgentRuntime> {
    let slots: Vec<String> = meta
        .roster()
        .iter()
        .map(|entry| entry.slot.clone())
        .collect();
    liveness::agent_runtimes(panes, &slots)
}

#[cfg(debug_assertions)]
thread_local! {
    static AFTER_CLASSIFY: std::cell::Cell<Option<fn(&std::path::Path)>> =
        const { std::cell::Cell::new(None) };
}

/// Arm a callback between classification and [`listing::Presentation::enter`].
#[cfg(debug_assertions)]
pub fn set_after_classify_hook(hook: Option<fn(&std::path::Path)>) {
    AFTER_CLASSIFY.with(|slot| slot.set(hook));
}

#[cfg(test)]
mod socket_spec;

#[cfg(test)]
mod world_spec;

#[cfg(test)]
mod tests {
    #[test]
    fn live_seat_socket_proof_is_cache_only_and_unknown_never_matches() {
        use crate::{inventory::ServerId, meta::Selector};
        let name = |s: &str| ServerId::Selected(Selector::Name(s.into()));
        let socket = |s: &str| ServerId::Selected(Selector::Socket(s.into()));
        let (recorded, alias) = (name("ae"), name("alias"));
        let no_read = |_: &ServerId| -> Option<String> {
            panic!("new socket read");
        };
        let mut cache = super::SocketPaths::primed(no_read, &recorded, Some("/tmp/ae".into()));
        cache.seen.push((alias.clone(), Some("/tmp/ae".into())));
        assert!(cache.known_same(&recorded, &alias));
        assert!(cache.known_same(&recorded, &socket("/tmp/ae")));
        assert!(!cache.known_same(&recorded, &socket("/private/tmp/ae")));
        assert!(!cache.known_same(&recorded, &socket("/tmp/other")));
        assert!(!cache.known_same(&name("uncached"), &name("uncached")));
        assert!(!cache.known_same(&socket("/tmp/new"), &socket("/tmp/new")));
        cache
            .seen
            .push((socket("/tmp/ae"), Some("/tmp/elsewhere".into())));
        assert!(!cache.known_same(&recorded, &socket("/tmp/ae")));
        cache.seen.push((name("failed"), None));
        assert!(!cache.known_same(&recorded, &name("failed")));
        assert!(!cache.known_same(&name("failed"), &alias));
        cache.seen.push((socket("/tmp/missing"), None));
        assert!(!cache.known_same(&recorded, &socket("/tmp/missing")));
    }

    fn aliased_socket_of(server: &crate::inventory::ServerId) -> Option<String> {
        match server {
            crate::inventory::ServerId::Selected(crate::meta::Selector::Name(name))
                if name == crate::doors::DEFAULT_SERVER_NAME =>
            {
                Some("/private/tmux/ae".to_owned())
            }
            crate::inventory::ServerId::Selected(crate::meta::Selector::Socket(path)) => {
                Some(path.display().to_string())
            }
            _ => None,
        }
    }

    fn named(name: &str) -> crate::inventory::ServerId {
        crate::inventory::ServerId::Selected(crate::meta::Selector::Name(name.to_owned()))
    }

    #[test]
    fn a_recorded_socket_suppresses_the_default_name_only_when_tmux_proves_the_alias() {
        let recorded = crate::inventory::ServerId::Selected(crate::meta::Selector::Socket(
            "/private/tmux/ae".into(),
        ));
        let default = named(crate::doors::DEFAULT_SERVER_NAME);
        let unresolved = crate::inventory::ServerId::Ambient;
        let mut sockets = super::SocketPaths::asking(aliased_socket_of);
        let candidates = vec![recorded.clone(), default.clone(), unresolved.clone()];
        assert_eq!(
            sockets.deduplicated(candidates),
            [recorded.clone(), unresolved],
            "the durable spelling wins; an unproved server is never discarded"
        );
        assert!(sockets.equivalent(&recorded, &default));
    }

    /// Every candidate keeps its runtime, in snapshot order, and every Running
    /// record is in the pane map even when nothing could be listed for it.
    #[test]
    fn each_running_record_has_a_pane_entry_and_every_candidate_a_runtime() {
        use crate::digest::Status;
        let classified = |name: &str, status| crate::liveness::Classified {
            candidate: crate::inventory::Candidate {
                name: name.to_owned(),
                durable: Some(crate::inventory::DurableRecord {
                    path: std::path::PathBuf::from(format!("/ae-no-such-root/{name}")),
                    name: name.to_owned(),
                    layout: crate::inventory::Layout::Canonical,
                    server: crate::meta::ServerSelector::Missing,
                    meta_read: crate::session::MetaRead::Absent,
                    snapshot: crate::session::RecordSnapshot::default(),
                }),
                live: None,
            },
            status,
        };
        let snapshot = crate::liveness::Snapshot {
            sessions: vec![
                classified("up", Status::Running),
                classified("down", Status::Stopped),
            ],
            incomplete: Vec::new(),
        };
        let (runtimes, panes, _) = super::observed_runtimes(&snapshot);
        let statuses: Vec<Status> = runtimes.iter().map(|runtime| runtime.status).collect();
        assert_eq!(statuses, [Status::Running, Status::Stopped]);
        let listed = std::path::PathBuf::from("/ae-no-such-root/up");
        assert_eq!(panes, super::ListedPanes::from([(listed, None)]));
    }

    /// R1: both phases of one read consume ONE answer per server, a failure
    /// stays a failure, and two servers are asked independently.
    #[test]
    fn a_fleet_read_asks_each_server_once_and_a_failure_stays_failed() {
        use crate::inventory::{DiscoveredSession, Discovery, QueryFailed, RecordedOn, ServerId};
        use crate::meta::Selector;
        use std::cell::RefCell;
        struct Counting {
            asked: RefCell<Vec<ServerId>>,
            dead: ServerId,
        }
        impl Discovery for Counting {
            fn enumerate(&self, server: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed> {
                self.asked.borrow_mut().push(server.clone());
                if *server == self.dead {
                    return Err(QueryFailed);
                }
                Ok(vec![DiscoveredSession {
                    name: "up".to_owned(),
                    marker: Some("up".to_owned()),
                }])
            }
        }
        let live = ServerId::Selected(Selector::Socket("/ae-no-such/live.sock".into()));
        let dead = ServerId::Selected(Selector::Socket("/ae-no-such/dead.sock".into()));
        let inner = Counting {
            asked: RefCell::new(Vec::new()),
            dead: dead.clone(),
        };
        let recorded = RecordedOn::default();
        let discovery = super::FleetDiscovery {
            inner: &inner,
            default_is_optional: false,
            recorded: &recorded,
            boot: None,
            now: 0,
            settled: RefCell::default(),
        };
        for _ in 0..2 {
            assert_eq!(
                discovery.enumerate(&live),
                Ok(vec![DiscoveredSession {
                    name: "up".to_owned(),
                    marker: Some("up".to_owned()),
                }])
            );
            assert_eq!(discovery.enumerate(&dead), Err(QueryFailed));
        }
        assert_eq!(
            *inner.asked.borrow(),
            [live, dead],
            "each server is asked once, in first-asked order; a failure is not retried"
        );
    }
}
