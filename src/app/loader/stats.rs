//! The Statistics tab's stepped read: one job lists, each later one reads one
//! source, so a close or a due fleet read waits at most one source. Reads only:
//! no store, no lock, no temp file, no process.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use crate::quota::Budget;
use crate::stats::{Fold, Source, SourceGap, View};

/// How much one scan may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Limits {
    pub(super) sources: usize,
    pub(super) bytes: u64,
    pub(super) journal: u64,
}

/// The product's bounds.
pub(super) const LIMITS: Limits = Limits {
    sources: 512,
    bytes: 256 * 1024 * 1024,
    journal: 4 * 1024 * 1024,
};

/// One source still to read.
enum Pending {
    Live(String, PathBuf),
    Archive(crate::archive::KeptArchive),
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
            fold: Some(Fold::default()),
            queue: None,
        }
    }

    /// One job's work: `Some` once the last source is in.
    pub(super) fn step(&mut self) -> Option<View> {
        let fold = self.fold.as_mut()?;
        if let Some(queue) = &mut self.queue {
            let next = queue.pop_front()?;
            let source = read(next, self.limits, &mut self.budget);
            fold.add(source);
        } else {
            self.queue = Some(self.list());
            if self.remaining().is_some_and(|left| left > 0) {
                return None;
            }
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

    /// Sources left to read, `None` before they are listed.
    pub(super) fn remaining(&self) -> Option<usize> {
        self.queue.as_ref().map(VecDeque::len)
    }

    pub(super) const fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(super) const fn limits(&self) -> Limits {
        self.limits
    }
}

/// One source's bytes, or the gap that stands for them. A failed source never
/// ends the scan.
fn read(pending: Pending, limits: Limits, budget: &mut Budget) -> Source {
    match pending {
        Pending::Live(name, dir) => {
            let meta = crate::usage::meta_bytes(&dir.join(crate::store::META), budget);
            match meta {
                Ok(meta) => {
                    let journal = dir.join(crate::store::EVENTS);
                    let journal = crate::usage::bounded_bytes(&journal, limits.journal, budget);
                    Source::live(&name, &meta, journal)
                }
                Err(gap) => Source::failed(&name, None, gap),
            }
        }
        Pending::Archive(kept) => {
            let crate::archive::KeptArchive { id, path, claimed } = kept;
            if claimed {
                return Source::failed(&id, Some(&id), SourceGap::Claimed);
            }
            match crate::archive::validated_journal(&path, &id) {
                Ok(journal) => {
                    let bytes = crate::usage::bounded_bytes(&journal, limits.journal, budget);
                    Source::archive(&id, bytes)
                }
                Err(_) => Source::failed(&id, Some(&id), SourceGap::Invalid),
            }
        }
    }
}
