//! The maintained store's upkeep, without a thread: list the sources, read
//! one chunk of one outside the lock, publish it under `stats.lock`, report
//! the step. The app's thread calls [`Maintainer::step`] and waits
//! [`pause`] between steps.
//!
//! Sources take turns: one that has more to read goes to the back after each
//! step, so a large journal never strands a later one, except that a line
//! longer than one read keeps its turn until it ends or is known to be over
//! the limit. A live tail with no LF yet is read once, in a step of its own,
//! and so is a quiet source whose witness had to be read again
//! ([`Outcome::Checked`]); until its journal's identity, length or change
//! times move, later rounds look at its status alone. A round that finds
//! nothing to read is [`Outcome::Idle`]. The first step, and every
//! idle one, first finishes what a crash left owed.
//!
//! A payload file met damaged or over its cap is checked again under the lock
//! by the payload reads alone, then repaired once: the payload is replaced in
//! a later epoch and every source read again. Damage met again before that
//! rebuild has caught up (a round that could not list every source has not)
//! stops this maintainer, refused, for the rest of its life; the next one (the
//! next `ae app` run) repairs once again. An IO error stops nothing.
//!
//! Each round also takes a census of what the store holds. A source listed
//! under another group than the one holding it moves: it is removed from the
//! old group first and registered on its next turn. After the listed
//! sources, every source the store holds that the round did not list is
//! removed, but only from a namespace the round listed completely, and then
//! groups a crash left empty are pruned. An archive is validated again only
//! when its tree's stat identity moved since a validation proved it valid.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::disk::{
    self, Census, Decision, FileJournals, Files, Held, Life, Limits, Next, Prepared, Refused,
    SourceSpec, Tail, Taken, Why,
};
use super::doc::Key;
use crate::quota::Budget;
use crate::stats::SourceGap;

/// The least wait after a round with nothing to read.
pub const REFRESH: Duration = Duration::from_secs(5);
/// The least wait after the lock was busy.
const BUSY: Duration = Duration::from_millis(250);
/// The least wait after any other step.
const BREATH: Duration = Duration::from_millis(10);

/// What one step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A read published: the source's cursor moved `from` → `to`.
    Published {
        from: u64,
        to: u64,
        r#gen: u64,
    },
    /// A line longer than one read held in memory, `from` its start.
    Holding {
        from: u64,
        to: u64,
    },
    /// A quiet source's journal checked against its witness at cursor `at`:
    /// nothing new.
    Checked {
        at: u64,
    },
    /// Prepared work dropped under the lock, nothing written.
    Discarded(Why),
    /// The lock was busy: nothing done.
    Deferred,
    /// Nothing to read anywhere.
    Idle,
    Refused(Refused),
    /// A damaged payload replaced by an empty one in the next epoch.
    Repaired {
        file: String,
        reason: String,
    },
    /// A vanished or moved source removed (and any emptied group pruned).
    Removed {
        r#gen: u64,
    },
    /// An emptied group's leftover files pruned.
    Pruned,
}

/// One step's report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub source: Option<String>,
    pub outcome: Outcome,
    /// New journal bytes read.
    pub read: u64,
    /// The whole attempt's wall time: listing, read, lock wait, publication.
    pub active: Duration,
    /// How long the lock was held.
    pub held: Duration,
}

/// The wait after `step`: never less than the step itself took, so the
/// upkeep holds at most about half a core. An IO error, which stops nothing,
/// waits as long as an idle round before it is met again; so does a snapshot
/// refusal, because the cap is fixed and warming the same payload again at
/// once would only meet it again (a hot warm loop).
#[must_use]
pub fn pause(step: &Step) -> Duration {
    let least = match step.outcome {
        Outcome::Idle | Outcome::Refused(Refused::Io(_) | Refused::Snapshot { .. }) => REFRESH,
        Outcome::Deferred => BUSY,
        _ => BREATH,
    };
    step.active.max(least)
}

/// Where one round over the sources is.
#[derive(Debug, Default)]
struct Round {
    /// The listing was taken.
    listed: bool,
    /// Something was read.
    worked: bool,
    /// The listing could not enumerate everything: a rebuild that has not
    /// seen every source has not caught up.
    incomplete: bool,
}

/// The upkeep of the store under one root.
#[derive(Debug)]
pub struct Maintainer {
    root: PathBuf,
    limits: Limits,
    lock_wait: Duration,
    files: Files,
    journals: FileJournals,
    queue: VecDeque<SourceSpec>,
    round: Round,
    /// The line held for the source at the queue's front.
    held: Option<Held>,
    /// The source the last prepared work is of, its time, its read and what
    /// the read before it was told.
    prepared: Option<(SourceSpec, Duration, u64, Option<Tail>)>,
    /// Each archive's verdict and the tree identity it was judged at: an
    /// archive is judged again only when that identity moves.
    judged: BTreeMap<String, Judged>,
    /// A repair's rebuild has not caught up yet.
    rebuilding: bool,
    /// Damage met again while rebuilding: the upkeep stopped.
    stopped: Option<Refused>,
    /// What the store held when the round was listed.
    census: Census,
    /// The ids the round listed.
    listed: BTreeSet<String>,
    /// Each listed archive's tree hash, when a validation proved that tree
    /// valid.
    fingerprints: BTreeMap<String, String>,
    /// The round's removals and prunes, done once its sources are.
    chores: VecDeque<Chore>,
    /// What each source's last read that found nothing new saw: an unchanged
    /// journal is passed over on its length alone.
    tails: BTreeMap<String, Tail>,
    /// The first step asked whether a crash left work owed.
    begun: bool,
    #[cfg(test)]
    recheck: Recheck,
    #[cfg(test)]
    validated: Recheck,
}

/// An archive's verdict at one tree identity.
#[derive(Debug, Clone)]
struct Judged {
    identity: String,
    verdict: Result<PathBuf, SourceGap>,
}

/// Work a round does under the lock once its sources are done.
#[derive(Debug, Clone)]
enum Chore {
    /// A source removed from `key` while it has `stamp`.
    Remove {
        key: Key,
        id: String,
        stamp: (u64, u64),
    },
    /// An empty group's files removed.
    Prune(Key),
}

/// A test's hook, run before the lock is requested for a damage recheck.
#[cfg(test)]
type Hook = Box<dyn FnMut(&Path) + Send>;

#[cfg(test)]
#[derive(Default)]
struct Recheck(Option<Hook>);

#[cfg(test)]
impl std::fmt::Debug for Recheck {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(if self.0.is_some() {
            "Recheck(hook)"
        } else {
            "Recheck"
        })
    }
}

impl Maintainer {
    #[must_use]
    pub fn new(root: &Path, limits: Limits, lock_wait: Duration) -> Self {
        Self {
            root: root.to_path_buf(),
            limits,
            lock_wait,
            files: Files::at(root),
            journals: FileJournals::at(root),
            queue: VecDeque::new(),
            round: Round::default(),
            held: None,
            prepared: None,
            judged: BTreeMap::new(),
            rebuilding: false,
            stopped: None,
            census: Census::default(),
            listed: BTreeSet::new(),
            fingerprints: BTreeMap::new(),
            chores: VecDeque::new(),
            tails: BTreeMap::new(),
            begun: false,
            #[cfg(test)]
            recheck: Recheck::default(),
            #[cfg(test)]
            validated: Recheck::default(),
        }
    }

    /// Run `hook` after an unlocked read was refused with damage and before
    /// the lock is requested for the recheck: another writer healing the
    /// store before this one gets the lock.
    #[cfg(test)]
    pub fn on_recheck(&mut self, hook: impl FnMut(&Path) + Send + 'static) {
        self.recheck = Recheck(Some(Box::new(hook)));
    }

    /// Run `hook` with an archive's path right after its full validation
    /// and before its tree identity is taken again: a tree that changes
    /// while it is validated.
    #[cfg(test)]
    pub fn on_validated(&mut self, hook: impl FnMut(&Path) + Send + 'static) {
        self.validated = Recheck(Some(Box::new(hook)));
    }

    /// Whether repeated damage stopped the upkeep.
    #[must_use]
    pub const fn stopped(&self) -> bool {
        self.stopped.is_some()
    }

    /// The next read, prepared outside the lock; a step that ends without one
    /// (idle, holding a line, refused, repaired) is the `Err`.
    ///
    /// # Errors
    ///
    /// The finished step when there is no work to publish.
    pub fn prepare(&mut self) -> Result<Prepared, Step> {
        let started = Instant::now();
        self.next_work(started).map(|(work, spec, read, prior)| {
            self.prepared = Some((spec, started.elapsed(), read, prior));
            work
        })
    }

    /// Publish prepared work under the lock, recovering first.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "a commit takes the prepared work, so one preparation commits once"
    )]
    pub fn commit(&mut self, work: Prepared) -> Step {
        let started = Instant::now();
        let (spec, before, read, prior) = match self.prepared.take() {
            Some((spec, spent, read, prior)) if spec.id() == work.source() => {
                (Some(spec), spent, read, prior)
            }
            _ => (None, Duration::ZERO, 0, None),
        };
        let mut step = self.publish(&work, spec, read, prior.as_ref());
        step.active = before.saturating_add(started.elapsed());
        step
    }

    /// Prepare, then publish. The first step, and every step that would
    /// report [`Outcome::Idle`], first finish what a crash left owed, under
    /// the lock, so recovery never waits for new journal work; `held`
    /// counts that lock too.
    pub fn step(&mut self) -> Step {
        let started = Instant::now();
        let mut held = Duration::ZERO;
        if !std::mem::replace(&mut self.begun, true) {
            let (settled, refused) = self.settle();
            held = settled;
            if let Some(outcome) = refused {
                return Step {
                    source: None,
                    outcome,
                    read: 0,
                    active: started.elapsed(),
                    held,
                };
            }
        }
        let mut step = match self.prepare() {
            Ok(work) => self.commit(work),
            Err(step) => step,
        };
        if step.outcome == Outcome::Idle {
            let (settled, refused) = self.settle();
            held = held.saturating_add(settled);
            if let Some(outcome) = refused {
                step.outcome = outcome;
            }
        }
        step.held = step.held.saturating_add(held);
        step.active = step.active.max(started.elapsed());
        step
    }

    /// Under the lock, when a crash left work owed: recovery applied whole,
    /// or the damage it met handled as any other. How long the lock was
    /// held, and the outcome when recovery was refused.
    fn settle(&mut self) -> (Duration, Option<Outcome>) {
        if !disk::owes(&mut self.files).unwrap_or(false) {
            return (Duration::ZERO, None);
        }
        let rebuilding = self.rebuilding;
        let mut held = Duration::ZERO;
        let taken = disk::transact(
            &self.root,
            &mut self.files,
            &mut self.journals,
            self.lock_wait,
            &mut held,
            |files, _| {
                recover(files)
                    .err()
                    .map(|refused| damaged(files, rebuilding, refused))
            },
        );
        let refused = match taken {
            Taken::Done(damage) => damage.map(|damage| self.after(damage)),
            Taken::Over { recorded, cap } => {
                Some(Outcome::Refused(Refused::Snapshot { recorded, cap }))
            }
            Taken::Unlocked(_) | Taken::Contended => None,
        };
        (held, refused)
    }

    fn finished(source: Option<String>, outcome: Outcome, read: u64, started: Instant) -> Step {
        Step {
            source,
            outcome,
            read,
            active: started.elapsed(),
            held: Duration::ZERO,
        }
    }

    fn next_work(
        &mut self,
        started: Instant,
    ) -> Result<(Prepared, SourceSpec, u64, Option<Tail>), Step> {
        if let Some(stopped) = &self.stopped {
            return Err(Self::finished(
                None,
                Outcome::Refused(stopped.clone()),
                0,
                started,
            ));
        }
        loop {
            if self.queue.is_empty()
                && let Some(chore) = self.chores.pop_front()
            {
                return Err(self.chore(chore, started));
            }
            if self.queue.is_empty() {
                if self.round.listed && !self.round.worked {
                    return Err(self.idle(started));
                }
                self.queue = self.list().into();
                (self.round.listed, self.round.worked) = (true, false);
                if self.queue.is_empty() && self.chores.is_empty() {
                    return Err(self.idle(started));
                }
            }
            let Some(spec) = self.queue.pop_front() else {
                continue;
            };
            let id = spec.id().to_owned();
            // Held under another group: out of that one first, so the source
            // never has two owners at a commit.
            if let Some(chore) = self.moved(&spec) {
                self.queue.push_front(spec);
                self.round.worked = true;
                return Err(self.chore(chore, started));
            }
            let held = self.held.take().filter(|held| held.source() == id);
            let life = self.life(&spec);
            let prepared = Prepared::read_life(
                &mut self.files,
                &mut self.journals,
                &spec,
                self.limits,
                held,
                self.tails.get(&id),
                &life,
            );
            let prepared =
                prepared.map(|seen| (seen.next, seen.read, seen.touched, seen.cursor, seen.tail));
            match prepared {
                Ok((Next::Work(work), more, ..)) => {
                    let prior = self.tails.remove(&id);
                    self.round.worked = true;
                    return Ok((work, spec, more, prior));
                }
                Ok((Next::Holding(held), more, ..)) => {
                    self.tails.remove(&id);
                    self.round.worked = true;
                    let (from, to) = held.span();
                    self.held = Some(held);
                    self.queue.push_front(spec);
                    return Err(Self::finished(
                        Some(id),
                        Outcome::Holding { from, to },
                        more,
                        started,
                    ));
                }
                Ok((Next::Nothing, more, touched, at, tail)) => {
                    let pending = tail.as_ref().map(Tail::span);
                    match tail {
                        Some(tail) => self.tails.insert(id.clone(), tail),
                        None => self.tails.remove(&id),
                    };
                    // A live tail with no LF yet, read once: its own step, so
                    // one step reads one source's chunk at most.
                    if more > 0 {
                        self.round.worked = true;
                        let (from, to) = pending.unwrap_or_default();
                        return Err(Self::finished(
                            Some(id),
                            Outcome::Holding { from, to },
                            more,
                            started,
                        ));
                    }
                    // A quiet source whose journal had to be read to check
                    // its witness: its own step too. Nothing new was read, so
                    // the round can still end idle.
                    if touched {
                        return Err(Self::finished(
                            Some(id),
                            Outcome::Checked { at },
                            0,
                            started,
                        ));
                    }
                }
                Err(refused) => return Err(self.refused_step(spec, refused, started)),
            }
        }
    }

    /// The step of a preparation that was refused: a source deferred by a
    /// busy recheck keeps its turn.
    fn refused_step(&mut self, spec: SourceSpec, refused: Refused, started: Instant) -> Step {
        let (outcome, held) = self.refused_unlocked(&spec, refused);
        let id = spec.id().to_owned();
        if outcome == Outcome::Deferred {
            self.queue.push_front(spec);
        }
        let mut step = Self::finished(Some(id), outcome, 0, started);
        step.held = held;
        step
    }

    /// The round found nothing to read. A rebuild has caught up only when
    /// the round saw every source.
    fn idle(&mut self, started: Instant) -> Step {
        self.round.listed = false;
        if !self.round.incomplete {
            self.rebuilding = false;
        }
        Self::finished(None, Outcome::Idle, 0, started)
    }

    /// Every source, in order: live sessions by name, then kept archives. A
    /// root that would not enumerate is one failed source named for it, as
    /// the loader names it, never an empty answer.
    fn list(&mut self) -> Vec<SourceSpec> {
        // A census that cannot be taken proves nothing absent: the round
        // moves and removes nothing, and its reads meet the damage.
        let census = disk::census(&mut self.files, &self.census);
        let known = census.is_ok();
        self.census = census.unwrap_or_default();
        self.fingerprints.clear();
        let (specs, live, archives) = self.listed_specs();
        self.listed = specs.iter().map(|spec| spec.id().to_owned()).collect();
        self.tails.retain(|id, _| self.listed.contains(id));
        self.judged
            .retain(|id, _| self.listed.contains(&format!("archive/{id}")));
        self.chores.clear();
        if known {
            let whole = |id: &str| {
                if id == "sessions" || id.starts_with("live/") {
                    live
                } else if id == "archive" || id.starts_with("archive/") {
                    archives
                } else {
                    false
                }
            };
            for (id, entry) in &self.census.sources {
                if !self.listed.contains(id) && whole(id) {
                    self.chores.push_back(Chore::Remove {
                        key: entry.key.clone(),
                        id: id.clone(),
                        stamp: entry.stamp,
                    });
                }
            }
            for key in &self.census.empty {
                self.chores.push_back(Chore::Prune(key.clone()));
            }
        }
        specs
    }

    /// The removal from its old group that a source listed under another
    /// group needs first.
    fn moved(&self, spec: &SourceSpec) -> Option<Chore> {
        let entry = self.census.sources.get(spec.id())?;
        (entry.key != spec.key()).then(|| Chore::Remove {
            key: entry.key.clone(),
            id: spec.id().to_owned(),
            stamp: entry.stamp,
        })
    }

    /// The lifecycle inputs to preparing `spec`: its archive's proven tree
    /// hash, and the sources of its group whose evidence a read from 0 may
    /// copy: those the round did not list (a renamed session), and, for an
    /// archive, the live sessions it was made from.
    fn life(&self, spec: &SourceSpec) -> Life {
        let key = spec.key();
        let archive = matches!(spec, SourceSpec::Archive { .. });
        let donors = self
            .census
            .sources
            .iter()
            .filter(|(id, entry)| {
                entry.key == key
                    && entry.counted
                    && id.as_str() != spec.id()
                    && (!self.listed.contains(id.as_str()) || (archive && id.starts_with("live/")))
            })
            .map(|(id, _)| id.clone())
            .collect();
        Life {
            fingerprint: self.fingerprints.get(spec.id()).cloned(),
            donors,
        }
    }

    /// A removal or a prune under the lock, recovering first.
    fn chore(&mut self, chore: Chore, started: Instant) -> Step {
        let source = match &chore {
            Chore::Remove { id, .. } => Some(id.clone()),
            Chore::Prune(_) => None,
        };
        let rebuilding = self.rebuilding;
        let mut held = Duration::ZERO;
        let taken = disk::transact(
            &self.root,
            &mut self.files,
            &mut self.journals,
            self.lock_wait,
            &mut held,
            |files, _| {
                let decided = recover(files).and_then(|()| match &chore {
                    Chore::Remove { key, id, stamp } => {
                        disk::remove(files, key.as_deref(), id, Some(*stamp))
                    }
                    Chore::Prune(key) => disk::prune(files, key.as_deref()).map(Decision::Publish),
                });
                match decided {
                    Ok(Decision::Publish(plan)) => match plan.apply(files, plan.ops.len()) {
                        Ok(()) => Ok(plan.generation()),
                        Err(why) => Err(Damage::Plain(Refused::Io(why.to_string()))),
                    },
                    Ok(Decision::Discard(why)) => Err(Damage::Discarded(why)),
                    Err(refused) => Err(damaged(files, rebuilding, refused)),
                }
            },
        );
        let outcome = match taken {
            Taken::Unlocked(why) => {
                let outcome = if why.kind() == std::io::ErrorKind::WouldBlock {
                    Outcome::Deferred
                } else {
                    Outcome::Refused(Refused::Io(format!("stats.lock: {why}")))
                };
                if !matches!(chore, Chore::Remove { .. }) || self.queue.is_empty() {
                    self.chores.push_front(chore);
                }
                // An earlier stale attempt's hold still counts.
                let mut step = Self::finished(source, outcome, 0, started);
                step.held = held;
                return step;
            }
            Taken::Contended => {
                if !matches!(chore, Chore::Remove { .. }) || self.queue.is_empty() {
                    self.chores.push_front(chore);
                }
                let mut step = Self::finished(source, Outcome::Deferred, 0, started);
                step.held = held;
                return step;
            }
            Taken::Over { recorded, cap } => Outcome::Refused(Refused::Snapshot { recorded, cap }),
            Taken::Done(Ok(generation)) => {
                if let Chore::Remove { id, .. } = &chore {
                    self.census.sources.remove(id);
                }
                match chore {
                    Chore::Remove { .. } => Outcome::Removed {
                        r#gen: generation.unwrap_or_default(),
                    },
                    Chore::Prune(_) => Outcome::Pruned,
                }
            }
            Taken::Done(Err(damage)) => {
                if let Chore::Remove { id, .. } = &chore {
                    self.census.sources.remove(id);
                }
                self.after(damage)
            }
        };
        let mut step = Self::finished(source, outcome, 0, started);
        step.held = held;
        step
    }

    /// The round's sources, and whether the live sessions and the archives
    /// were each listed completely.
    fn listed_specs(&mut self) -> (Vec<SourceSpec>, bool, bool) {
        let mut specs = Vec::new();
        let roots = crate::inventory::Roots::under(&self.root);
        let scan = crate::inventory::durable_meta_records(&roots);
        let lost = !scan.incomplete.is_empty();
        self.round.incomplete = lost;
        if lost {
            specs.push(failed("sessions"));
        }
        let mut records = scan.records;
        records.sort_by(|left, right| left.name.cmp(&right.name));
        records.dedup_by(|left, right| left.name == right.name);
        for record in records {
            let id = format!("live/{}", record.name);
            let mut budget = Budget::local(u64::MAX);
            match crate::usage::meta_bytes(&record.path.join(crate::store::META), &mut budget) {
                Ok(meta) => {
                    self.journals
                        .track(&id, record.path.join(crate::store::EVENTS));
                    specs.push(SourceSpec::Live {
                        id,
                        label: record.name,
                        meta,
                    });
                }
                Err(gap) => specs.push(SourceSpec::Failed {
                    id,
                    label: record.name,
                    session: None,
                    gap,
                }),
            }
        }
        let Ok(kept) = crate::archive::kept_archives(&self.root) else {
            self.round.incomplete = true;
            specs.push(failed("archive"));
            return (specs, !lost, false);
        };
        for kept in kept {
            let id = format!("archive/{}", kept.id);
            if kept.claimed {
                specs.push(SourceSpec::Failed {
                    id,
                    label: kept.id.clone(),
                    session: Some(kept.id),
                    gap: SourceGap::Claimed,
                });
                continue;
            }
            match self.judge(&id, &kept) {
                Ok(journal) => {
                    self.journals.track(&id, journal);
                    specs.push(SourceSpec::Archive { id, uuid: kept.id });
                }
                Err(gap) => specs.push(SourceSpec::Failed {
                    id,
                    label: kept.id.clone(),
                    session: Some(kept.id),
                    gap,
                }),
            }
        }
        (specs, !lost, true)
    }

    /// An archive's journal once its whole tree proves valid. The verdict
    /// holds while the tree's stat identity does: judged at this identity
    /// before, or stored with the counted row as the identity a validation
    /// proved, it is not validated again. A validation counts only when the
    /// identity is the same after it as before; a valid tree's identity is
    /// what the round tells the archive's preparation to store.
    fn judge(
        &mut self,
        id: &str,
        kept: &crate::archive::KeptArchive,
    ) -> Result<PathBuf, SourceGap> {
        // A tree with no identity proves nothing valid: its gap, or Invalid.
        let unproved = |verdict: Result<PathBuf, SourceGap>| verdict.and(Err(SourceGap::Invalid));
        let Ok(before) = crate::archive::tree_identity(&kept.path) else {
            self.judged.remove(&kept.id);
            note_validation(&kept.id);
            return unproved(validate(kept));
        };
        let stored = self
            .census
            .sources
            .get(id)
            .filter(|entry| entry.counted)
            .and_then(|entry| entry.fp.as_deref())
            == Some(before.as_str());
        let verdict = match self.judged.get(&kept.id) {
            Some(judged) if judged.identity == before => judged.verdict.clone(),
            _ if stored => Ok(kept.path.join(crate::store::EVENTS)),
            _ => {
                note_validation(&kept.id);
                let verdict = validate(kept);
                #[cfg(test)]
                if let Some(hook) = self.validated.0.as_mut() {
                    hook(&kept.path);
                }
                // The tree moved while it was validated: nothing is proved
                // this round, and nothing is kept.
                if crate::archive::tree_identity(&kept.path).ok().as_deref()
                    != Some(before.as_str())
                {
                    self.judged.remove(&kept.id);
                    return unproved(verdict);
                }
                verdict
            }
        };
        self.judged.insert(
            kept.id.clone(),
            Judged {
                identity: before.clone(),
                verdict: verdict.clone(),
            },
        );
        if verdict.is_ok() {
            self.fingerprints.insert(id.to_owned(), before);
        }
        verdict
    }

    fn publish(
        &mut self,
        work: &Prepared,
        spec: Option<SourceSpec>,
        read: u64,
        prior: Option<&Tail>,
    ) -> Step {
        let source = Some(work.source().to_owned());
        let (from, to) = work.span();
        let rebuilding = self.rebuilding;
        let mut held = Duration::ZERO;
        let taken = disk::transact(
            &self.root,
            &mut self.files,
            &mut self.journals,
            self.lock_wait,
            &mut held,
            |files, journals| {
                let decided = recover(files).and_then(|()| disk::publish(files, journals, work));
                match decided {
                    Ok(Decision::Publish(plan)) => match plan.apply(files, plan.ops.len()) {
                        Ok(()) => Ok(plan.generation().unwrap_or_default()),
                        Err(why) => Err(Damage::Plain(Refused::Io(why.to_string()))),
                    },
                    Ok(Decision::Discard(why)) => Err(Damage::Discarded(why)),
                    Err(refused) => Err(damaged(files, rebuilding, refused)),
                }
            },
        );
        let outcome = match taken {
            Taken::Unlocked(why) => {
                let busy = why.kind() == std::io::ErrorKind::WouldBlock;
                if let Some(spec) = spec {
                    self.queue.push_front(spec);
                }
                let outcome = if busy {
                    Outcome::Deferred
                } else {
                    Outcome::Refused(Refused::Io(format!("stats.lock: {why}")))
                };
                // An earlier stale attempt's hold still counts.
                let mut step = Self::finished(source, outcome, read, Instant::now());
                step.held = held;
                return step;
            }
            Taken::Contended => {
                if let Some(spec) = spec {
                    self.queue.push_front(spec);
                }
                Outcome::Deferred
            }
            Taken::Over { recorded, cap } => Outcome::Refused(Refused::Snapshot { recorded, cap }),
            Taken::Done(Ok(generation)) => {
                if work.more()
                    && let Some(spec) = spec
                {
                    self.queue.push_back(spec);
                }
                // Consumed to its end: the next round looks at the journal's
                // status alone. Taken after the lock is released: it reads
                // the journal's status.
                if let Some(tail) = work.settled(generation, &mut self.journals, prior) {
                    self.tails.insert(work.source().to_owned(), tail);
                }
                Outcome::Published {
                    from,
                    to,
                    r#gen: generation,
                }
            }
            Taken::Done(Err(Damage::Discarded(why))) => {
                // Owned elsewhere, or gone: the next round's census and
                // listing decide again. Anything else is tried again.
                if !matches!(why, Why::Owned | Why::Gone)
                    && let Some(spec) = spec
                {
                    self.queue.push_back(spec);
                }
                Outcome::Discarded(why)
            }
            Taken::Done(Err(damage)) => self.after(damage),
        };
        Step {
            source,
            outcome,
            read,
            active: Duration::ZERO,
            held,
        }
    }

    /// A refusal met while preparing `spec`: the damage is met again under
    /// the lock, after recovery, by the payload reads alone, before anything
    /// is repaired. Healed meanwhile, the source is prepared again on the
    /// next step, outside the lock. The outcome, and how long the lock was
    /// held.
    fn refused_unlocked(&mut self, spec: &SourceSpec, refused: Refused) -> (Outcome, Duration) {
        if refused.damage().is_none() {
            return (Outcome::Refused(refused), Duration::ZERO);
        }
        #[cfg(test)]
        if let Some(hook) = self.recheck.0.as_mut() {
            hook(&self.root);
        }
        let rebuilding = self.rebuilding;
        let mut held = Duration::ZERO;
        let taken = disk::transact(
            &self.root,
            &mut self.files,
            &mut self.journals,
            self.lock_wait,
            &mut held,
            |files, _| {
                recover(files)
                    .and_then(|()| disk::verify(files, spec))
                    .err()
                    .map(|refused| damaged(files, rebuilding, refused))
            },
        );
        let outcome = match taken {
            Taken::Done(Some(damage)) => self.after(damage),
            Taken::Over { recorded, cap } => Outcome::Refused(Refused::Snapshot { recorded, cap }),
            Taken::Done(None) | Taken::Unlocked(_) | Taken::Contended => Outcome::Deferred,
        };
        (outcome, held)
    }

    /// What the lock found, applied to the upkeep: damage repaired once per
    /// rebuild; met again before the rebuild caught up, the upkeep stops.
    fn after(&mut self, damage: Damage) -> Outcome {
        match damage {
            Damage::Plain(refused) => Outcome::Refused(refused),
            Damage::Discarded(why) => Outcome::Discarded(why),
            Damage::Again(refused) | Damage::Unrepaired(refused) => {
                self.stopped = Some(refused.clone());
                Outcome::Refused(refused)
            }
            Damage::Repaired { file, reason } => {
                self.rebuilding = true;
                self.queue.clear();
                self.tails.clear();
                self.chores.clear();
                self.census = Census::default();
                (self.held, self.round.listed, self.round.worked) = (None, false, false);
                Outcome::Repaired { file, reason }
            }
        }
    }
}

/// What a step under the lock met besides its own result.
#[derive(Debug)]
enum Damage {
    /// A refusal that stops nothing: not damage, or an IO error repairing it.
    Plain(Refused),
    /// Prepared work dropped.
    Discarded(Why),
    /// Damage met again while rebuilding.
    Again(Refused),
    /// A repair refused for another reason than IO.
    Unrepaired(Refused),
    Repaired {
        file: String,
        reason: String,
    },
}

/// Under the lock: finish what a crash left.
fn recover(files: &mut Files) -> Result<(), Refused> {
    let plan = disk::recovery(files)?;
    if plan.ops.is_empty() {
        return Ok(());
    }
    plan.apply(files, plan.ops.len())
        .map_err(|why| Refused::Io(why.to_string()))
}

/// Under the lock: `refused` met; damage repaired once per rebuild (the
/// exception: a repair reads under the lock).
fn damaged(files: &mut Files, rebuilding: bool, refused: Refused) -> Damage {
    let Some((file, reason)) = refused.damage() else {
        return Damage::Plain(refused);
    };
    if rebuilding {
        return Damage::Again(refused);
    }
    let repaired = files.exception(|files| {
        disk::repair(files).and_then(|plan| {
            plan.apply(files, plan.ops.len())
                .map_err(|why| Refused::Io(why.to_string()))
        })
    });
    match repaired {
        Ok(()) => Damage::Repaired { file, reason },
        // An IO error is not the damage met again: the next step asks again.
        Err(why @ Refused::Io(_)) => Damage::Plain(why),
        Err(why) => Damage::Unrepaired(why),
    }
}

/// The failed source a root that would not enumerate stands for.
fn failed(root: &str) -> SourceSpec {
    SourceSpec::Failed {
        id: root.to_owned(),
        label: root.to_owned(),
        session: None,
        gap: SourceGap::Unreadable,
    }
}

#[cfg(test)]
thread_local! {
    static VALIDATIONS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The archive ids this thread validated in full since the last call.
#[cfg(test)]
#[must_use]
pub fn take_validations() -> Vec<String> {
    VALIDATIONS.with(|seen| std::mem::take(&mut *seen.borrow_mut()))
}

/// Counts a full archive validation for [`take_validations`].
#[cfg(test)]
fn note_validation(id: &str) {
    VALIDATIONS.with(|seen| seen.borrow_mut().push(id.to_owned()));
}

/// Nothing outside tests counts validations.
#[cfg(not(test))]
const fn note_validation(_id: &str) {}

/// An archive's journal once its whole tree proves valid.
fn validate(kept: &crate::archive::KeptArchive) -> Result<PathBuf, SourceGap> {
    let mut refused = SourceGap::Invalid;
    let mut content = |file: &Path| {
        let mut budget = Budget::local(u64::MAX);
        crate::usage::bounded_bytes(file, ARCHIVE_MEMBER, &mut budget).map_err(|gap| {
            if matches!(gap, SourceGap::Oversized | SourceGap::Budget) {
                refused = gap;
            }
            std::io::Error::other(gap.word())
        })
    };
    crate::archive::validated_journal(&kept.path, &kept.id, &mut content).map_err(|_| refused)
}

/// The most bytes one archive member read whole may hold: meta, digest.
const ARCHIVE_MEMBER: u64 = 4 << 20;
