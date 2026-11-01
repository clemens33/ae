//! The Statistics tab's fold: recorded worker spawns by configured route.
//!
//! Pure: journal bytes in, a [`View`] out, no IO.

/// Why a source gave no bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceGap {
    Missing,
    Unreadable,
    Oversized,
    Budget,
    Invalid,
    Claimed,
}

/// One journal as the reader found it.
pub struct Source;

impl Source {
    #[must_use]
    pub fn live(_name: &str, _meta: &[u8], _journal: Result<Vec<u8>, SourceGap>) -> Self {
        Self
    }

    #[must_use]
    pub fn archive(_id: &str, _journal: Result<Vec<u8>, SourceGap>) -> Self {
        Self
    }

    #[must_use]
    pub fn failed(_label: &str, _session: Option<&str>, _gap: SourceGap) -> Self {
        Self
    }
}

/// The sources read so far.
#[derive(Default)]
pub struct Fold;

impl Fold {
    pub fn add(&mut self, _source: Source) {}

    #[must_use]
    pub fn finish(self, _this: Option<&str>) -> View {
        View {
            all: Scoped::default(),
            this: ThisSession::NoUuid,
        }
    }
}

/// Fold every source.
#[must_use]
pub fn fold(sources: Vec<Source>, this: Option<&str>) -> View {
    let mut fold = Fold::default();
    sources.into_iter().for_each(|source| fold.add(source));
    fold.finish(this)
}

/// Both scopes of one read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub all: Scoped,
    pub this: ThisSession,
}

/// The settings session's scope, when it has a session id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThisSession {
    Ready(Box<Scoped>),
    NoUuid,
}

/// One scope's counts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scoped {
    pub total: u64,
    pub legacy: u64,
    pub model_known: u64,
    pub effort_known: u64,
    pub range: Option<(String, String)>,
    pub sources_read: u64,
    pub sources_total: u64,
    pub harness: Vec<Row>,
    pub client: Vec<Row>,
    pub model_effort: Vec<Row>,
    pub joint: Vec<Row>,
    pub gaps: Vec<Gap>,
}

impl Scoped {
    #[must_use]
    pub const fn balanced(&self) -> bool {
        false
    }

    #[must_use]
    pub const fn partial(&self) -> bool {
        false
    }
}

/// One table row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub count: u64,
}

/// One table cell.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cell {
    Value(String),
    NoneSet,
    Direct,
    Unproved,
    Unknown,
}

/// One coverage line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub source: Option<String>,
    pub reason: Reason,
    pub count: u64,
}

impl Gap {
    #[must_use]
    pub fn line(&self) -> String {
        String::new()
    }
}

/// Why a coverage line exists.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    Source(SourceGap),
    Malformed,
    Damaged,
    Unsupported,
    CertificateLost,
    Conflict,
    SessionMismatch,
    NoSessionId,
    LiveNoBirth,
    LegacyExcluded { sessions: u64, a: u64, b: u64 },
    Unproved { model: bool, word: String },
}

#[cfg(test)]
#[path = "stats_spec.rs"]
mod spec;
