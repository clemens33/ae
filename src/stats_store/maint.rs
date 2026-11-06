//! The maintained store's upkeep, without a thread: list the sources, read
//! one chunk of one outside the lock, publish it under `stats.lock`, report
//! the step. The app's thread calls [`Maintainer::step`] and waits
//! [`pause`] between steps.
//!
//! Sources take turns: one that has more to read goes to the back after each
//! step, so a large journal never strands a later one, except that a line
//! longer than one read keeps its turn until it ends or is known to be over
//! the limit. A live tail with no LF yet is read once, in a step of its own;
//! until its journal changes, later rounds look at its length alone. A round
//! that finds nothing to read is [`Outcome::Idle`]. The first step, and every
//! idle one, first finishes what a crash left owed.
//!
//! A payload file met damaged or over its cap is checked again under the lock
//! by the payload reads alone, then repaired once: the payload is replaced in
//! a later epoch and every source read again. Damage met again before that
//! rebuild has caught up (a round that could not list every source has not)
//! stops this maintainer, refused, for the rest of its life; the next one (the
//! next `ae app` run) repairs once again. An IO error stops nothing.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::disk::{
    self, Decision, FileJournals, Files, Held, Limits, Next, Payload, Prepared, Refused,
    SourceSpec, Tail, Why,
};
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
/// waits as long as an idle round before it is met again.
#[must_use]
pub fn pause(step: &Step) -> Duration {
    let least = match step.outcome {
        Outcome::Idle | Outcome::Refused(Refused::Io(_)) => REFRESH,
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
    /// The source the last prepared work is of, its time and its read.
    prepared: Option<(SourceSpec, Duration, u64)>,
    /// Each archive's validated journal, or why it gave none: archives do not
    /// change.
    archives: BTreeMap<String, Result<PathBuf, SourceGap>>,
    /// A repair's rebuild has not caught up yet.
    rebuilding: bool,
    /// Damage met again while rebuilding: the upkeep stopped.
    stopped: Option<Refused>,
    /// What each source's last read that found nothing new saw: an unchanged
    /// journal is passed over on its length alone.
    tails: BTreeMap<String, Tail>,
    /// The first step asked whether a crash left work owed.
    begun: bool,
    #[cfg(test)]
    recheck: Recheck,
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
            archives: BTreeMap::new(),
            rebuilding: false,
            stopped: None,
            tails: BTreeMap::new(),
            begun: false,
            #[cfg(test)]
            recheck: Recheck::default(),
        }
    }

    /// Run `hook` after an unlocked read was refused with damage and before
    /// the lock is requested for the recheck: another writer healing the
    /// store before this one gets the lock.
    #[cfg(test)]
    pub fn on_recheck(&mut self, hook: impl FnMut(&Path) + Send + 'static) {
        self.recheck = Recheck(Some(Box::new(hook)));
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
        self.next_work(started).map(|(work, spec, read)| {
            self.prepared = Some((spec, started.elapsed(), read));
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
        let (spec, before, read) = match self.prepared.take() {
            Some((spec, spent, read)) if spec.id() == work.source() => (Some(spec), spent, read),
            _ => (None, Duration::ZERO, 0),
        };
        let mut step = self.publish(&work, spec, read);
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
        let Ok(lock) = Payload::at(&self.root).lock(self.lock_wait) else {
            return (Duration::ZERO, None);
        };
        let taken = Instant::now();
        let refused = self
            .recover()
            .err()
            .map(|refused| self.refused_locked(refused));
        let held = taken.elapsed();
        drop(lock);
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

    fn next_work(&mut self, started: Instant) -> Result<(Prepared, SourceSpec, u64), Step> {
        if let Some(stopped) = &self.stopped {
            return Err(Self::finished(
                None,
                Outcome::Refused(stopped.clone()),
                0,
                started,
            ));
        }
        loop {
            if self.queue.is_empty() {
                if self.round.listed && !self.round.worked {
                    return Err(self.idle(started));
                }
                self.queue = self.list().into();
                (self.round.listed, self.round.worked) = (true, false);
                if self.queue.is_empty() {
                    return Err(self.idle(started));
                }
            }
            let Some(spec) = self.queue.pop_front() else {
                continue;
            };
            let id = spec.id().to_owned();
            let held = self.held.take().filter(|held| held.source() == id);
            let prepared = Prepared::read_observed(
                &mut self.files,
                &mut self.journals,
                &spec,
                self.limits,
                held,
                self.tails.get(&id),
            );
            match prepared {
                Ok((Next::Work(work), more, _)) => {
                    self.tails.remove(&id);
                    self.round.worked = true;
                    return Ok((work, spec, more));
                }
                Ok((Next::Holding(held), more, _)) => {
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
                Ok((Next::Nothing, more, tail)) => {
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
                }
                Err(refused) => {
                    let (outcome, held) = self.refused_unlocked(&spec, refused);
                    if outcome == Outcome::Deferred {
                        self.queue.push_front(spec);
                    }
                    let mut step = Self::finished(Some(id), outcome, 0, started);
                    step.held = held;
                    return Err(step);
                }
            }
        }
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
        let specs = self.listed_specs();
        let ids: BTreeSet<&str> = specs.iter().map(SourceSpec::id).collect();
        self.tails.retain(|id, _| ids.contains(id.as_str()));
        specs
    }

    fn listed_specs(&mut self) -> Vec<SourceSpec> {
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
            return specs;
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
            let validated = self
                .archives
                .entry(kept.id.clone())
                .or_insert_with(|| validate(&kept))
                .clone();
            match validated {
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
        specs
    }

    fn publish(&mut self, work: &Prepared, spec: Option<SourceSpec>, read: u64) -> Step {
        let source = Some(work.source().to_owned());
        let (from, to) = work.span();
        let lock = match Payload::at(&self.root).lock(self.lock_wait) {
            Ok(lock) => lock,
            Err(why) => {
                let busy = why.kind() == std::io::ErrorKind::WouldBlock;
                if let Some(spec) = spec {
                    self.queue.push_front(spec);
                }
                let outcome = if busy {
                    Outcome::Deferred
                } else {
                    Outcome::Refused(Refused::Io(format!("stats.lock: {why}")))
                };
                return Self::finished(source, outcome, read, Instant::now());
            }
        };
        let taken = Instant::now();
        let decided = self
            .recover()
            .and_then(|()| disk::publish(&mut self.files, &mut self.journals, work));
        let outcome = match decided {
            Ok(Decision::Publish(plan)) => match plan.apply(&mut self.files, plan.ops.len()) {
                Ok(()) => {
                    if work.more()
                        && let Some(spec) = spec
                    {
                        self.queue.push_back(spec);
                    }
                    Outcome::Published {
                        from,
                        to,
                        r#gen: plan.generation().unwrap_or_default(),
                    }
                }
                Err(why) => Outcome::Refused(Refused::Io(why.to_string())),
            },
            Ok(Decision::Discard(why)) => {
                if let Some(spec) = spec {
                    self.queue.push_back(spec);
                }
                Outcome::Discarded(why)
            }
            Err(refused) => self.refused_locked(refused),
        };
        let held = taken.elapsed();
        drop(lock);
        Step {
            source,
            outcome,
            read,
            active: Duration::ZERO,
            held,
        }
    }

    /// Under the lock: finish what a crash left.
    fn recover(&mut self) -> Result<(), Refused> {
        let plan = disk::recovery(&mut self.files)?;
        plan.apply(&mut self.files, plan.ops.len())
            .map_err(|why| Refused::Io(why.to_string()))
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
        let Ok(lock) = Payload::at(&self.root).lock(self.lock_wait) else {
            return (Outcome::Deferred, Duration::ZERO);
        };
        let taken = Instant::now();
        let again = self
            .recover()
            .and_then(|()| disk::verify(&mut self.files, spec))
            .err();
        let outcome = match again {
            Some(refused) => self.refused_locked(refused),
            None => Outcome::Deferred,
        };
        let held = taken.elapsed();
        drop(lock);
        (outcome, held)
    }

    /// Under the lock: damage repaired once per rebuild; met again before the
    /// rebuild caught up, the upkeep stops.
    fn refused_locked(&mut self, refused: Refused) -> Outcome {
        let Some((file, reason)) = refused.damage() else {
            return Outcome::Refused(refused);
        };
        if self.rebuilding {
            self.stopped = Some(refused.clone());
            return Outcome::Refused(refused);
        }
        let repaired = disk::repair(&mut self.files).and_then(|plan| {
            plan.apply(&mut self.files, plan.ops.len())
                .map_err(|why| Refused::Io(why.to_string()))
        });
        match repaired {
            Ok(()) => {
                self.rebuilding = true;
                self.queue.clear();
                self.tails.clear();
                (self.held, self.round.listed, self.round.worked) = (None, false, false);
                Outcome::Repaired { file, reason }
            }
            // An IO error is not the damage met again: the next step asks again.
            Err(why @ Refused::Io(_)) => Outcome::Refused(why),
            Err(why) => {
                self.stopped = Some(why.clone());
                Outcome::Refused(why)
            }
        }
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
