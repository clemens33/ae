//! Adoption: the fleet strip of a session whose own watchdog is not running.

use super::{
    DETACHED_MOTION_TICK, Duration, Instant, Look, Mark, OptionScope, Path, PathBuf, Status, procs,
    theme, tmux, transport,
};
#[cfg(doc)]
use super::{MotionState, ticker::TickerMode};

// ---------------------------------------------------------------------------
// ADOPTION: the fleet strip of a session whose own watchdog is not running.
// ---------------------------------------------------------------------------

/// How often a daemon redraws the strips it has adopted.
///
/// Adoption's OWN cadence, deliberately not the ticker's: [`MotionState::step`]
/// draws nothing while no client is attached, and the peer being filled is
/// exactly the session somebody IS looking at. A detached adopter must keep
/// drawing, so this runs attached or not, in every [`TickerMode`].
pub(super) const ADOPTION_TICK: Duration = DETACHED_MOTION_TICK;

/// One session this daemon fills the fleet strip for, because nothing else is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Adopted {
    /// The session name, as ae's own records and the server agreed on it.
    pub(super) name: String,
    /// The `$<n>` the enumeration proved, and the ONLY target ever written to.
    /// An id, never a name: tmux never reuses one while the server runs, so a
    /// write cannot land on a session that took the name over since.
    pub(super) id: String,
    /// Its state directory — read for the pidfile, never written.
    pub(super) meta_dir: PathBuf,
    /// The watchdog pid this session's pidfile named at enumeration, `None`
    /// when it had none. The pause test compares against THIS rather than
    /// against presence: a dead daemon leaves its pidfile behind, and a target
    /// that paused on a stale file would never be filled at all.
    pub(super) enum_pid: Option<u32>,
    /// The strip text last published here — the write-on-change memory, one
    /// per target.
    pub(super) published: Option<String>,
}

/// Who this daemon is drawing the fleet strip for besides itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Adoption {
    /// The adopted sessions, refreshed once per verdict cycle.
    pub(super) targets: Vec<Adopted>,
    /// Every session ae's OWN records prove is a running ae session of this
    /// state root — adopted or not.
    ///
    /// The strip's rank rule cannot answer this: a session nobody measures
    /// publishes no rank, and a rank is a tmux option a stranger could set too.
    /// So the rows a rankless session is drawn in are admitted on ae's records
    /// and the ownership proof, never on the option.
    pub(super) known: Vec<String>,
}

/// Whether a session's own watchdog is running, from its pidfile and ONE
/// process table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WatchdogPresence {
    /// A pidfile naming a process the table lists.
    Live,
    /// No pidfile, or one naming a process the table does not list — with the
    /// pid it named, so a later tick can tell a lingering file from a new one.
    Absent(Option<u32>),
    /// There was no table to ask. Never adopted: the peer's own daemon may be
    /// right there.
    Unknown,
}

/// Read `meta_dir`'s pidfile and judge it against `table`.
///
/// READ-ONLY, deliberately not [`crate::watchdog_lifecycle::presence`]: that one
/// deletes a stale pidfile, and this is another session's state directory. The
/// pane proof that `presence` also makes is a tmux process per peer, which is
/// what this whole path exists to avoid; a pidfile plus the cycle's own table
/// is the same two facts minus the third.
fn watchdog_presence(meta_dir: &Path, table: Option<&[procs::Proc]>) -> WatchdogPresence {
    let Some(pid) = crate::watchdog_glue::read_pid(meta_dir) else {
        return WatchdogPresence::Absent(None);
    };
    match table {
        None => WatchdogPresence::Unknown,
        // ACCEPTED: a recycled pid makes a dead watchdog read as live, which
        // costs the peer its adopted strip until something reuses the pid no
        // longer. `watchdog stop`'s own stale-pidfile cleanup narrows it.
        Some(table) if table.iter().any(|proc| proc.pid == pid) => WatchdogPresence::Live,
        Some(_) => WatchdogPresence::Absent(Some(pid)),
    }
}

/// The liveness backend for the adoption scan: it answers only THIS server, and
/// only from the ownership proof already read for each name.
///
/// The picker's stopped rows are enumerated the same way and admit a live name
/// on ae's rank row; this one cannot, because the sessions it is looking for are
/// exactly the ones that publish no rank. So the marker IS
/// `seed_unwatched`'s proof — an `AE_SESSION` marker plus an `AE_HOME` naming
/// this state root — which is also what decides whether a peer may be written
/// to at all. One read, one rule, both questions.
pub(super) struct AdoptionBackend<'a> {
    pub(super) server: &'a crate::inventory::ServerId,
    pub(super) sockets: &'a crate::fleet_read::SocketPaths,
    pub(super) proven: &'a [crate::inventory::DiscoveredSession],
}

impl crate::inventory::Discovery for AdoptionBackend<'_> {
    fn enumerate(
        &self,
        server: &crate::inventory::ServerId,
    ) -> std::result::Result<Vec<crate::inventory::DiscoveredSession>, crate::inventory::QueryFailed>
    {
        if !self.sockets.equivalent(server, self.server) {
            return Err(crate::inventory::QueryFailed);
        }
        Ok(self.proven.to_vec())
    }
}

/// The ownership a peer must prove before this daemon writes a display fact
/// into it — `seed_unwatched`'s proof, and for the same reason.
///
/// A NAME is not an identity. The marker says a session is ae's; the home says
/// WHICH ae state root launched it. Without both, a stranger who took the name
/// over on a shared server would be handed this fleet's strip, and a session
/// belonging to another `AE_HOME` would be drawn into a fleet it is not part of.
pub(super) fn proven_ownership(
    owned: Option<&transport::SessionOwnership>,
    root: &Path,
) -> Option<String> {
    owned
        .filter(|owned| !owned.marker.is_empty() && Path::new(&owned.home) == root)
        .map(|owned| owned.marker.clone())
}

/// Who this daemon owes a fleet strip, once per verdict cycle.
///
/// `None` when there is no state root to scan, which is not evidence that
/// anything changed: the caller keeps the adoption it already had.
///
/// `table` is the cycle's OWN process snapshot, passed in rather than taken:
/// one `ps` per cycle regardless of how many peers there are, and none at all
/// on the 2 s tick.
pub(super) fn enumerate_adoption(
    server: &crate::inventory::ServerId,
    session: &str,
    listing: &[tmux::FleetListingRow],
    table: Option<&[procs::Proc]>,
    prior: &Adoption,
) -> Option<Adoption> {
    let root = crate::doors::state_root(crate::shape::current())?;
    // META ONLY: the picker's read. Identities and the `placed` rule, never a
    // session's journal — nothing here is derived from events.
    let scan = crate::inventory::durable_meta_records(&crate::inventory::Roots::under(&root));
    // The OWNERSHIP proof, made once per name this server is actually showing.
    // A record whose session is not on this server costs nothing, and neither
    // does this daemon's own: it is never a target and never vouched for, so
    // probing it would buy two tmux spawns a cycle and no answer anyone reads.
    let names: Vec<String> = scan
        .records
        .iter()
        .map(|record| record.name.clone())
        .filter(|name| name != session)
        .filter(|name| listing.iter().any(|row| &row.name == name))
        .collect();
    let read = transport::observe_peer_ownership(server, &names);
    let mut sockets = match read.socket {
        Some(path) => crate::fleet_read::SocketPaths::for_caller(server, Some(path)),
        None => crate::fleet_read::SocketPaths::live(),
    };
    // Warm the cache ONLY for a spelling that is not already ours: two records
    // naming the same server the same way need no tmux probe to be equivalent.
    for candidate in &scan.records {
        if let Some(selector) = candidate.server.entitles() {
            let recorded = crate::inventory::ServerId::Selected(selector.clone());
            if &recorded != server {
                let _ = sockets.proven_same(server, &recorded);
            }
        }
    }
    let proven: Vec<crate::inventory::DiscoveredSession> = read
        .peers
        .into_iter()
        .map(|(name, owned)| crate::inventory::DiscoveredSession {
            marker: proven_ownership(owned.as_ref(), &root),
            name,
        })
        .collect();
    let backend = AdoptionBackend {
        server,
        sockets: &sockets,
        proven: &proven,
    };
    let inventory = crate::inventory::Inventory {
        candidates: scan
            .records
            .into_iter()
            .map(crate::inventory::Candidate::durable)
            .collect(),
        // Carried, never recomputed: an incomplete scan is a REPORT, not a
        // refusal. The sessions ae could enumerate are adopted; the ones it
        // could not are simply not targets this cycle.
        incomplete: scan.incomplete,
    };
    Some(adoption_from(
        crate::liveness::classify(inventory, &backend).sessions,
        session,
        listing,
        prior,
        |meta_dir| watchdog_presence(meta_dir, table),
    ))
}

/// The adoption a classified scan decides — the whole rule, with no world in it.
///
/// RUNNING only, never this session itself, never a candidate the server is not
/// currently showing, and never one whose own watchdog is live or unproven. An
/// `unknown` candidate is simply not a target: the classifier says `unknown`
/// exactly when it could not prove the session, and an incomplete scan is
/// therefore adopt-the-known, skip-the-unknown rather than a refusal.
pub(super) fn adoption_from(
    classified: Vec<crate::liveness::Classified>,
    session: &str,
    listing: &[tmux::FleetListingRow],
    prior: &Adoption,
    presence: impl Fn(&Path) -> WatchdogPresence,
) -> Adoption {
    let mut next = Adoption::default();
    for classified in classified {
        if classified.status != Status::Running {
            continue;
        }
        let Some(record) = classified.candidate.durable else {
            continue;
        };
        // This daemon's own session is never a target: it publishes its own
        // strip, and a second writer would fight it every tick.
        if record.name == session {
            continue;
        }
        next.known.push(record.name.clone());
        let Some(row) = listing.iter().find(|row| row.name == record.name) else {
            continue;
        };
        let WatchdogPresence::Absent(enum_pid) = presence(&record.path) else {
            continue;
        };
        next.targets.push(Adopted {
            name: record.name.clone(),
            id: row.id.clone(),
            meta_dir: record.path,
            enum_pid,
            // Carried across the re-enumeration so a target that has not
            // changed is not rewritten once a minute for nothing.
            published: prior
                .targets
                .iter()
                .find(|held| held.id == row.id && held.name == record.name)
                .and_then(|held| held.published.clone()),
        });
    }
    next
}

/// The rows one strip draws, from ONE listing.
///
/// A ranked row draws its rank. A rankless row draws [`Mark::Stale`] when ae's
/// own records vouch for it — that is the rule "a session that RUNS is always a
/// row, and a row nobody is measuring says so", and it holds on every strip this
/// daemon writes, its own included. A rankless row nothing vouches for is
/// DROPPED: it is a session ae did not create, and the strip draws no strangers.
pub(super) fn fleet_rows(
    listing: &[tmux::FleetListingRow],
    known: &[String],
    current: &str,
) -> Vec<theme::FleetRow> {
    listing
        .iter()
        .filter_map(|row| {
            let mark = match row.rank.as_deref() {
                Some(rank) => Mark::from_rank(rank),
                None if known.iter().any(|name| name == &row.name) => Mark::Stale,
                None => return None,
            };
            Some(theme::FleetRow {
                name: row.name.clone(),
                id: row.id.clone(),
                mark,
                current: row.name == current,
            })
        })
        .collect()
}

/// The strips this daemon owes its adopted peers — and NOTHING else.
///
/// Every write this returns sets [`theme::FLEET_STRIP_OPTION`] on a peer, and
/// the pin beside it turns red if a second option ever joins them. That is the
/// whole of the fourth writer's licence: a rank, a glyph, a health segment or a
/// roster written here would be this daemon vouching for a session it is not
/// measuring.
///
/// Each strip is drawn STATIC, in the TARGET's look, with the TARGET as the
/// current row — a strip that cannot show you where you are is not a map — and
/// in the fleet order every writer shares.
pub(super) fn adoption_writes(
    adoption: &mut Adoption,
    listing: &[tmux::FleetListingRow],
    order: &theme::FleetOrder,
) -> Vec<tmux::OptionWrite> {
    let Adoption { targets, known } = adoption;
    let mut writes = Vec::new();
    for target in targets.iter_mut() {
        // The OWNER is back. Stop writing and let it publish over this; a
        // pidfile naming the SAME pid as at enumeration is the dead daemon's
        // leftover, not a live one, and must not pause the adoption.
        let pid = crate::watchdog_glue::read_pid(&target.meta_dir);
        if pid.is_some() && pid != target.enum_pid {
            continue;
        }
        // Proven again from THIS listing, by id and name together: a target
        // that has gone is skipped rather than written to, and a look ae could
        // not read is a target ae leaves alone this tick.
        let Some(row) = listing
            .iter()
            .find(|row| row.id == target.id && row.name == target.name)
        else {
            continue;
        };
        let look = Look::read(
            &row.look.icons,
            &row.look.palette,
            &row.look.drawn,
            &row.look.motion,
        );
        // `theme = off` on the TARGET does not stop this: ae fills `@ae_*` on
        // an undrawn session too, so a hand-written `status-right` still has
        // the strip to put in it.
        let strip = theme::fleet_strip(
            &look,
            &fleet_rows(listing, known, &target.name),
            None,
            order,
        );
        if target.published.as_deref() == Some(&strip) {
            continue;
        }
        writes.push(tmux::OptionWrite::new(
            OptionScope::Session,
            &target.id,
            theme::FLEET_STRIP_OPTION,
            &strip,
        ));
        target.published = Some(strip);
    }
    writes
}

/// Whether the adoption cadence is due now, stamping it when it is.
pub(super) fn adoption_due(last: &mut Option<Instant>, now: Instant) -> bool {
    if last.is_some_and(|stamp| now.duration_since(stamp) < ADOPTION_TICK) {
        return false;
    }
    *last = Some(now);
    true
}

/// Draw THIS session's row of a fleet listing with the rank this cycle
/// decided, whatever the listing caught of the published option.
///
/// The cycle sends its writes as one batch at its end, so the listing it reads
/// still carries LAST cycle's rank. The rank is the only cycle-written fact the
/// listing feeds back into the strip and the adoption scan. A peer's row is never
/// touched: this daemon does not vouch for another session's verdict, and a
/// listing without this session's id gains no row.
pub(super) fn overlay_current_rank(
    rows: &mut [tmux::FleetListingRow],
    session_id: &str,
    rank: Mark,
) {
    for row in rows.iter_mut().filter(|row| row.id == session_id) {
        row.rank = Some(rank.rank().to_string());
    }
}
