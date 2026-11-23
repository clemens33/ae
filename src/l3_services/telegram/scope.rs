//! The outbound session scope, resolved against each pass's census and inventory.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::routing::{RunningSession, Seat, World};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Orchestrator,
    All,
}

impl Mode {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "orchestrator" => Some(Self::Orchestrator),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Selector {
    Id(String),
    Name(String),
}

impl Selector {
    pub(super) fn list(value: &str) -> Option<Vec<Self>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        value
            .split(',')
            .map(|item| Self::parse(item.trim()))
            .collect()
    }

    fn parse(item: &str) -> Option<Self> {
        let uuid = crate::archive::canonical_uuid(item);
        if uuid.is_empty() {
            crate::config::is_session_name(item).then(|| Self::Name(item.to_owned()))
        } else {
            Some(Self::Id(uuid))
        }
    }

    fn matches(&self, session: &RunningSession) -> bool {
        match self {
            Self::Id(uuid) => session.session_id.eq_ignore_ascii_case(uuid),
            Self::Name(name) => session.name == *name,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Scope {
    pub(super) mode: Mode,
    pub(super) also: Vec<Selector>,
}

impl Scope {
    /// Directories whose full chat is in scope. `None` admits every running session.
    pub(super) fn admitted(
        &self,
        world: &dyn World,
        running: &[RunningSession],
    ) -> Option<BTreeSet<PathBuf>> {
        if self.mode == Mode::All {
            return None;
        }
        let Seat::Held { dir, .. } = world.seat() else {
            return None;
        };
        self.admitted_in(&dir, running)
    }

    /// [`Self::admitted`] against a seat already proved: `seat_dir` and the
    /// `running` snapshot it was proved in, with no census read of its own.
    /// `None` only in `all` mode.
    pub(super) fn admitted_in(
        &self,
        seat_dir: &Path,
        running: &[RunningSession],
    ) -> Option<BTreeSet<PathBuf>> {
        if self.mode == Mode::All {
            return None;
        }
        let mut dirs = BTreeSet::from([seat_dir.to_path_buf()]);
        for selector in &self.also {
            let mut matches = running.iter().filter(|session| selector.matches(session));
            if let (Some(session), None) = (matches.next(), matches.next()) {
                dirs.insert(session.dir.clone());
            }
        }
        Some(dirs)
    }
}
