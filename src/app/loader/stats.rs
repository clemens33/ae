//! The Statistics tab's stepped read.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// How much one scan may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Limits {
    pub(super) sources: usize,
    pub(super) bytes: u64,
    pub(super) journal: u64,
}

/// The product's bounds.
pub(super) const LIMITS: Limits = Limits {
    sources: 0,
    bytes: 0,
    journal: 0,
};

/// One scan.
pub(super) struct Scan {
    generation: u64,
    limits: Limits,
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
        _root: PathBuf,
        _live: BTreeMap<String, PathBuf>,
        _scanned: bool,
        generation: u64,
        _this: Option<String>,
        limits: Limits,
    ) -> Self {
        Self { generation, limits }
    }

    pub(super) fn step(&mut self) -> Option<crate::stats::View> {
        None
    }

    pub(super) const fn remaining(&self) -> Option<usize> {
        None
    }

    pub(super) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) const fn limits(&self) -> Limits {
        self.limits
    }
}
