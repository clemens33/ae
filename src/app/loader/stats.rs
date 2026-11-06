//! The Statistics tab's stepped read: one job lists, each later one reads one
//! chunk of one source, so a close or a due fleet read waits at most that
//! chunk. A journal is streamed line by line into the fold and never held, with
//! no size cutoff of its own: it is limited by the scan's byte budget, and what
//! the fold keeps of it by the retained-state budget. A journal bigger than
//! `chunk` stays open between steps. Reads only: no store, no lock, no temp
//! file, no process. The store `ae app` maintains meanwhile (`app::maintainer`)
//! is neither read nor waited on until S5 cuts the tab over. Every byte of
//! content, a source's meta or an archive's validation included, comes under
//! one budget; a directory listing has no bound, so one enormous archive costs
//! its own step that long.
//!
//! A step reads at most `chunk` journal bytes. The first step of a source adds
//! its open work: a live meta (at most 1 MiB) or an archive's validation (at
//! most two `member`s), each counted in the same budget.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::quota::Budget;
use crate::stats::{Fold, Reading, Source, SourceGap, View};
use crate::usage::{Journal, Line};

/// How much one scan may read and keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Limits {
    /// Sources one scan reads; the rest are turned away with a named gap.
    pub(super) sources: usize,
    /// Content bytes the whole scan may read: meta, validation and journals.
    pub(super) bytes: u64,
    /// The most content bytes one journal line may hold, LF excluded; a longer
    /// line is counted and discarded, the journal still read.
    pub(super) line: usize,
    /// The most bytes one archive member read whole may hold: meta, digest.
    pub(super) member: u64,
    /// Journal bytes one step reads.
    pub(super) chunk: u64,
    /// Charged bytes the fold may keep across the whole scan.
    pub(super) retained: u64,
}

/// The product's bounds.
pub(super) const LIMITS: Limits = Limits {
    sources: 512,
    bytes: 256 * 1024 * 1024,
    line: 4 * 1024 * 1024,
    member: 4 * 1024 * 1024,
    chunk: 4 * 1024 * 1024,
    retained: 64 * 1024 * 1024,
};

/// One source still to read.
enum Pending {
    Live(String, PathBuf),
    Archive(crate::archive::KeptArchive),
}

/// A journal open between steps, with what has been read of it.
struct Open {
    reading: Reading,
    journal: Journal,
}

/// One scan.
pub(super) struct Scan {
    root: PathBuf,
    live: BTreeMap<String, PathBuf>,
    scanned: bool,
    generation: u64,
    this: Option<String>,
    limits: Limits,
    budget: Budget,
    fold: Option<Fold>,
    /// `None` until the first step lists the sources.
    queue: Option<VecDeque<Pending>>,
    /// The journal a step left unfinished.
    current: Option<Open>,
}

impl Scan {
    pub(super) fn new(
        root: PathBuf,
        live: BTreeMap<String, PathBuf>,
        scanned: bool,
        generation: u64,
        this: Option<String>,
    ) -> Self {
        Self::limited(root, live, scanned, generation, this, LIMITS)
    }

    pub(super) fn limited(
        root: PathBuf,
        live: BTreeMap<String, PathBuf>,
        scanned: bool,
        generation: u64,
        this: Option<String>,
        limits: Limits,
    ) -> Self {
        Self {
            root,
            live,
            scanned,
            generation,
            this,
            limits,
            budget: Budget::local(limits.bytes),
            fold: Some(Fold::with_retained(limits.retained)),
            queue: None,
            current: None,
        }
    }

    /// One job's work: `Some` once the last source is in.
    pub(super) fn step(&mut self) -> Option<View> {
        let fold = self.fold.as_mut()?;
        if let Some(open) = self.current.take() {
            self.current = advance(open, self.limits, fold);
        } else if let Some(queue) = &mut self.queue {
            let next = queue.pop_front()?;
            self.current = begin(next, self.limits, &mut self.budget, fold);
        } else {
            self.queue = Some(self.list());
        }
        if self.remaining().is_some_and(|left| left > 0) {
            return None;
        }
        let fold = self.fold.take()?;
        Some(fold.finish(self.this.as_deref()))
    }

    /// Every source to read, in order: live names, then archive ids. What the
    /// cap turns away and what cannot be listed goes straight into the fold.
    fn list(&mut self) -> VecDeque<Pending> {
        let mut gaps = Vec::new();
        if !self.scanned {
            gaps.push(Source::failed("sessions", None, SourceGap::Unreadable));
        }
        let archives = crate::archive::kept_archives(&self.root).unwrap_or_else(|_| {
            gaps.push(Source::failed("archive", None, SourceGap::Unreadable));
            Vec::new()
        });
        let live = std::mem::take(&mut self.live);
        let mut queue: VecDeque<Pending> = live
            .into_iter()
            .map(|(name, dir)| Pending::Live(name, dir))
            .chain(archives.into_iter().map(Pending::Archive))
            .collect();
        let over = queue.split_off(self.limits.sources.min(queue.len()));
        gaps.extend(over.into_iter().map(|turned| match turned {
            Pending::Live(name, _) => Source::failed(&name, None, SourceGap::Budget),
            Pending::Archive(kept) => Source::failed(&kept.id, Some(&kept.id), SourceGap::Budget),
        }));
        if let Some(fold) = &mut self.fold {
            for source in gaps {
                fold.add(source);
            }
        }
        queue
    }

    /// Sources left to read, the one open included; `None` before they are
    /// listed.
    pub(super) fn remaining(&self) -> Option<usize> {
        let queued = self.queue.as_ref().map(VecDeque::len)?;
        Some(queued + usize::from(self.current.is_some()))
    }

    pub(super) const fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(super) const fn limits(&self) -> Limits {
        self.limits
    }
}

/// Open one source and read its first chunk. A source that fails to open never
/// ends the scan: its gap goes into the fold and the next step moves on.
fn begin(pending: Pending, limits: Limits, budget: &mut Budget, fold: &mut Fold) -> Option<Open> {
    match pending {
        Pending::Live(name, dir) => {
            let meta = match crate::usage::meta_bytes(&dir.join(crate::store::META), budget) {
                Ok(meta) => meta,
                Err(gap) => return refuse(fold, Source::failed(&name, None, gap)),
            };
            let reading = match Reading::live(&name, &meta, fold.room()) {
                Ok(reading) => reading,
                Err(refused) => return refuse(fold, refused),
            };
            let journal = dir.join(crate::store::EVENTS);
            start(reading, &journal, limits, budget, fold)
        }
        Pending::Archive(kept) => {
            let crate::archive::KeptArchive { id, path, claimed } = kept;
            if claimed {
                return refuse(fold, Source::failed(&id, Some(&id), SourceGap::Claimed));
            }
            let mut refused = SourceGap::Invalid;
            let mut content = |file: &Path| {
                crate::usage::bounded_bytes(file, limits.member, budget).map_err(|gap| {
                    if matches!(gap, SourceGap::Oversized | SourceGap::Budget) {
                        refused = gap;
                    }
                    std::io::Error::other(gap.word())
                })
            };
            let Ok(journal) = crate::archive::validated_journal(&path, &id, &mut content) else {
                return refuse(fold, Source::failed(&id, Some(&id), refused));
            };
            let reading = match Reading::archive(&id, fold.room()) {
                Ok(reading) => reading,
                Err(refused) => return refuse(fold, refused),
            };
            start(reading, &journal, limits, budget, fold)
        }
    }
}

/// A source that gave nothing goes into the fold and nothing stays open.
fn refuse(fold: &mut Fold, source: Source) -> Option<Open> {
    fold.add(source);
    None
}

/// Open `journal` for `reading` and read its first chunk.
fn start(
    reading: Reading,
    journal: &Path,
    limits: Limits,
    budget: &mut Budget,
    fold: &mut Fold,
) -> Option<Open> {
    match crate::usage::open_journal(journal, limits.line, budget) {
        Ok(journal) => advance(Open { reading, journal }, limits, fold),
        Err(gap) => refuse(fold, reading.fail(gap)),
    }
}

/// Read one more chunk of an open journal. Done, it goes into the fold; a gap
/// or the retained-state budget gives the whole source up, none of its lines
/// kept; otherwise it stays open for the next step.
fn advance(open: Open, limits: Limits, fold: &mut Fold) -> Option<Open> {
    let Open {
        mut reading,
        mut journal,
    } = open;
    let mut over_budget = false;
    let read = journal.pump(limits.chunk, &mut |line| {
        if over_budget {
            return;
        }
        match line {
            Line::Text(text) => over_budget = reading.line(text).is_err(),
            Line::Over => reading.over_long(),
        }
    });
    if over_budget {
        return refuse(fold, reading.fail(SourceGap::Retained));
    }
    match read {
        Ok(true) => {
            fold.add_reading(reading);
            None
        }
        Ok(false) => Some(Open { reading, journal }),
        Err(gap) => refuse(fold, reading.fail(gap)),
    }
}
