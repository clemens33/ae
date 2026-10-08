//! What the reader keeps between fleet reads: each session's parsed journal
//! and memo, reused while the file's [`Identity`] is the one it had when it
//! was read, and read whole again by today's readers on any change. A file
//! that is not a regular file, or whose read failed, is never kept.
//!
//! Nothing here reads a file itself: the identities come from the store, the
//! bytes from [`SessionRead::open`] and [`store::SessionStore::memo_bytes`].

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::brief::{self, Filed};
use crate::session::{RecordSnapshot, SessionRead};
use crate::store::{self, Identity};

/// The parsed journals and memos of the sessions the last fleet read listed.
#[derive(Default)]
pub(super) struct Journals {
    held: BTreeMap<PathBuf, Held>,
    /// Bytes handed to the journal parser.
    #[cfg(test)]
    parsed: u64,
    /// Bytes handed to the memo parser.
    #[cfg(test)]
    memo_parsed: u64,
}

/// One session directory's kept files, each with the identity it was read at.
#[derive(Default)]
struct Held {
    events: Option<(Identity, SessionRead)>,
    memo: Option<(Identity, Vec<Filed>)>,
}

impl Journals {
    /// [`RecordSnapshot::read`] of `dir`, its journal reused while unchanged.
    pub(super) fn snapshot(&mut self, dir: &Path) -> RecordSnapshot {
        RecordSnapshot::read_with(dir, || self.events(dir))
    }

    /// The journal of `dir` as [`SessionRead::open`] reads it now.
    fn events(&mut self, dir: &Path) -> io::Result<SessionRead> {
        // Taken BEFORE the read: a write after it changes the next identity.
        let identity = store::open(dir).events_identity();
        let held = self.held.entry(dir.to_path_buf()).or_default();
        if let (Some(now), Some((then, read))) = (identity, &held.events)
            && now == *then
        {
            return Ok(read.clone());
        }
        let read = SessionRead::open(dir);
        held.events = identity.zip(read.as_ref().ok().cloned());
        #[cfg(test)]
        if read.is_ok() {
            self.parsed += identity.map_or(0, Identity::bytes);
        }
        read
    }

    /// The memo of `dir`, folded as the reader folds it now.
    pub(super) fn memo(&mut self, dir: &Path) -> io::Result<Vec<Filed>> {
        let store = store::open(dir);
        let identity = store.memo_identity();
        let held = self.held.entry(dir.to_path_buf()).or_default();
        if let (Some(now), Some((then, filed))) = (identity, &held.memo)
            && now == *then
        {
            return Ok(filed.clone());
        }
        let filed = store.memo_bytes().map(|bytes| {
            #[cfg(test)]
            {
                self.memo_parsed += bytes.len() as u64;
            }
            brief::filed(&bytes)
        });
        held.memo = identity.zip(filed.as_ref().ok().cloned());
        filed
    }

    /// Keep only the sessions under `dirs`.
    pub(super) fn retain(&mut self, dirs: &[PathBuf]) {
        self.held.retain(|dir, _| dirs.contains(dir));
    }

    /// Bytes handed to the journal and memo parsers since [`Journals::default`].
    #[cfg(test)]
    pub(super) fn costs(&self) -> (u64, u64) {
        (self.parsed, self.memo_parsed)
    }

    /// The session directories an entry is held for.
    #[cfg(test)]
    pub(super) fn cached_dirs(&self) -> std::collections::BTreeSet<PathBuf> {
        self.held.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::super::fleet_spec::{self, Files};
    use super::Journals;
    use crate::brief::Filed;
    use crate::session::RecordSnapshot;

    impl Files for Journals {
        fn snapshot(&mut self, dir: &Path) -> RecordSnapshot {
            Journals::snapshot(self, dir)
        }
        fn memo(&mut self, dir: &Path) -> io::Result<Vec<Filed>> {
            Journals::memo(self, dir)
        }
        fn costs(&self) -> (u64, u64) {
            Journals::costs(self)
        }
        fn retain(&mut self, dirs: &[PathBuf]) {
            Journals::retain(self, dirs);
        }
        fn cached_dirs(&self) -> BTreeSet<PathBuf> {
            Journals::cached_dirs(self)
        }
    }

    #[test]
    fn a_kept_journal_and_memo_answer_every_change_as_a_full_read() {
        fleet_spec::file_changes(Journals::default);
    }

    #[test]
    fn a_nanosecond_mtime_tells_same_length_rewrites_apart() {
        fleet_spec::file_identity(Journals::default);
    }

    #[test]
    fn absent_nonregular_and_unreadable_files_keep_their_meanings() {
        fleet_spec::file_shapes(Journals::default);
    }

    #[test]
    fn an_unchanged_file_is_parsed_once_and_a_changed_one_whole() {
        fleet_spec::file_costs(Journals::default);
    }

    #[test]
    fn only_the_sessions_retained_are_kept() {
        fleet_spec::file_eviction(Journals::default);
    }
}
