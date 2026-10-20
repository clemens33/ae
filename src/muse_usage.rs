//! The Muse quota door: the opt-in, the held answer every ae process shares, and
//! the one paid ask behind them.
//!
//! OPT-IN: nothing here asks unless the GLOBAL config says
//! `[workspace] muse_quota = on`. A project `.ae/config` never votes, so a
//! checkout cannot make ae spend an account's quota.
//!
//! MUSE HAS NO FREE LOCAL SOURCE. The only read of the allowance is one model
//! call (`muse exec`, about 32K input tokens), so the ask is rationed by ONE
//! held record under the state root, whatever it answered: a failure costs
//! tokens too, and a timeout may already have spent them. A record stands for
//! [`muse::TTL_SECS`] across every process; only the process that takes the
//! zero-wait lock asks, and every precondition for recording the answer is
//! proved BEFORE the ask, because a call that cannot be recorded would be paid
//! again by the next process.
//!
//! The paid answer is carried in [`Refreshed`], so a failed publish never turns
//! into a guess or a re-read: the caller shows what was just paid for, and says
//! the record was not kept. An absent `muse` binary asks nothing and is not
//! recorded.

use std::io;
use std::path::Path;
use std::time::Duration;

use crate::quota::muse::{self, Cached, Outcome};
use crate::transport::MuseRun;

/// The global `[workspace]` key that opts in.
pub const KEY_SETTING: &str = "muse_quota";

/// The cache directory under the state root, and its two files.
const DIR: &str = "quota";
const CACHE: &str = "muse.json";
const LOCK: &str = ".muse.lock";
/// The most cache record a reader holds.
const MAX_CACHE: u64 = 64 * 1024;

/// What the global config says about asking Muse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// No opt-in: absent, or exactly `off`.
    Off,
    /// Exactly `on`.
    On,
    /// A value that is neither, or one the reader refused: treated as off.
    Unusable,
}

/// The global config's `muse_quota`: exactly `on` or `off`, absent is `off`,
/// anything else is [`Setting::Unusable`]. No project config is read.
#[must_use]
pub fn setting(global: Option<&Path>) -> Setting {
    let Some(global) = global else {
        return Setting::Off;
    };
    match crate::config::read_global_workspace_key(global, KEY_SETTING) {
        Ok(None) => Setting::Off,
        Ok(Some(value)) => match value.as_str() {
            "on" => Setting::On,
            "off" => Setting::Off,
            _ => Setting::Unusable,
        },
        Err(_) => Setting::Unusable,
    }
}

/// Why no answer could be asked for, or recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// The state root cannot hold the record, so an ask would be paid twice.
    NoStore,
    /// Another process holds the lock and is asking.
    Busy,
    /// No `muse` on `PATH`.
    Absent,
    /// The ask was paid and answered, and the record could not be published.
    Unrecorded,
}

impl Gap {
    /// The fixed words beside the rows, for an operator.
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::NoStore => "muse quota needs a writable state root",
            Self::Busy => "another ae quota is asking muse",
            Self::Absent => "muse not found on PATH",
            Self::Unrecorded => "muse answer not recorded: needs a writable state root",
        }
    }
}

/// What the cache holds for one Muse config home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// No record for this home: none yet, another home's, or damaged.
    Missing,
    /// The record written for this home.
    Cached(Cached),
}

/// The held record for `home` under `root`, read without asking anything.
#[must_use]
pub fn read(root: &Path, home: &Path) -> Read {
    match (read_cache(root), home.to_str()) {
        (Some(cached), Some(home)) if cached.home == home => Read::Cached(cached),
        _ => Read::Missing,
    }
}

/// What one refresh did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refreshed {
    /// Whether a model call was made.
    pub asked: bool,
    /// Why nothing was asked, or what was not recorded.
    pub gap: Option<Gap>,
    /// The record of the ask this call made, built before it was published;
    /// `None` when nothing was asked.
    pub answer: Option<Cached>,
}

impl Refreshed {
    const fn skipped(gap: Option<Gap>) -> Self {
        Self {
            asked: false,
            gap,
            answer: None,
        }
    }
}

/// Refresh the held record for `config_home` under `root` when it is due,
/// asking Muse as the account `operator_home` names.
#[must_use]
pub fn refresh(root: &Path, config_home: &Path, operator_home: &Path, now: i64) -> Refreshed {
    refresh_with(root, config_home, now, || {
        crate::transport::run_muse_quota(operator_home)
    })
}

/// [`refresh`] with the ask handed in.
///
/// Order, every precondition before any spend: the held record's verdict, the
/// directory proved real (a link is refused before the lock can follow it), the
/// zero-wait lock, the verdict again under it, a file proved creatable in that
/// directory, then the ask, and the record is published last. A held, busy or
/// unrecordable state asks nothing; there is no retry inside one call.
pub(crate) fn refresh_with(
    root: &Path,
    config_home: &Path,
    now: i64,
    ask: impl FnOnce() -> MuseRun,
) -> Refreshed {
    let Some(home) = config_home.to_str() else {
        return Refreshed::skipped(Some(Gap::NoStore));
    };
    if !due(root, home, now) {
        return Refreshed::skipped(None);
    }
    let dir = root.join(DIR);
    if ensure_dir(&dir).is_err() {
        return Refreshed::skipped(Some(Gap::NoStore));
    }
    let lock = match crate::store::lock(&dir.join(LOCK), Duration::ZERO) {
        Ok(lock) => lock,
        Err(why) if why.kind() == io::ErrorKind::WouldBlock => {
            return Refreshed::skipped(Some(Gap::Busy));
        }
        Err(_) => return Refreshed::skipped(Some(Gap::NoStore)),
    };
    if !due(root, home, now) {
        return Refreshed::skipped(None);
    }
    if crate::store::prove_publishable(&dir).is_err() {
        return Refreshed::skipped(Some(Gap::NoStore));
    }
    let (outcome, allowance) = match ask() {
        MuseRun::Absent => return Refreshed::skipped(Some(Gap::Absent)),
        MuseRun::Done { text } => match muse::extract(text.as_bytes()) {
            Ok(allowance) => (Outcome::Done, Some(allowance)),
            Err(_) => (Outcome::Unparsed, None),
        },
        MuseRun::TimedOut => (Outcome::TimedOut, None),
        MuseRun::TooLarge => (Outcome::TooLarge, None),
        MuseRun::Exited(code) => (Outcome::Exited(code), None),
        MuseRun::NotUtf8 => (Outcome::NotUtf8, None),
        MuseRun::Failed => (Outcome::Failed, None),
    };
    let record = Cached {
        home: home.to_owned(),
        attempt_at: now,
        outcome,
        allowance,
    };
    let kept = crate::store::publish_private_in(&dir, CACHE, record.render().as_bytes()).is_ok();
    drop(lock);
    Refreshed {
        asked: true,
        gap: (!kept).then_some(Gap::Unrecorded),
        answer: Some(record),
    }
}

/// Whether an ask is due for `home`: no usable record, or one past its hold.
fn due(root: &Path, home: &str, now: i64) -> bool {
    read_cache(root).is_none_or(|cached| cached.due(home, now))
}

/// Make the cache directory when it is absent and prove what is there is a
/// real directory: a link would let the lock be taken in its target while the
/// publication refuses it.
fn ensure_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(why) if why.kind() != io::ErrorKind::AlreadyExists => Err(why),
        _ => crate::store::prove_real_directory(dir),
    }
}

fn read_cache(root: &Path) -> Option<Cached> {
    match crate::store::read_capped_in(&root.join(DIR), CACHE, MAX_CACHE) {
        Ok(crate::store::SourceRead::Ready(bytes)) => muse::parse_cache(&bytes),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::PathBuf;

    const HOME: &str = "/synthetic/muse-home";
    const NOW: i64 = 1_792_000_000;
    const STREAM: &str = r#"{"record_type":"status","payload_type":"run.usage","payload":{"allowance":{"tier":null,"window":{"used_percent":12.5,"resets_at_ms":1792003600000},"weekly":{"used_percent":81,"resets_at_ms":1792400000000},"as_of_ms":1791999900000,"source":"stream"}}}"#;

    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-muse-usage-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A counted fake leg: how many times a model call would have been paid.
    fn asking(calls: &Cell<u32>, run: MuseRun) -> impl FnOnce() -> MuseRun + '_ {
        move || {
            calls.set(calls.get() + 1);
            run
        }
    }

    fn done() -> MuseRun {
        MuseRun::Done {
            text: STREAM.to_owned(),
        }
    }

    #[test]
    fn an_answered_ask_is_held_and_not_paid_again_inside_the_ttl() {
        let root = scratch("held");
        let calls = Cell::new(0);
        let first = refresh_with(&root, Path::new(HOME), NOW, asking(&calls, done()));
        assert!(first.asked && first.gap.is_none());
        assert_eq!(calls.get(), 1);
        assert!(matches!(read(&root, Path::new(HOME)), Read::Cached(_)));
        let second = refresh_with(&root, Path::new(HOME), NOW + 60, asking(&calls, done()));
        assert!(!second.asked && second.gap.is_none() && second.answer.is_none());
        assert_eq!(calls.get(), 1, "a held record must not be paid again");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_quota_directory_that_is_a_link_is_refused_before_any_spend_or_lock() {
        let root = scratch("linked");
        let target = scratch("linked-target");
        std::os::unix::fs::symlink(&target, root.join(DIR)).unwrap();
        let calls = Cell::new(0);
        let out = refresh_with(&root, Path::new(HOME), NOW, asking(&calls, done()));
        assert_eq!(calls.get(), 0, "a directory nothing can be published to");
        assert_eq!((out.asked, out.gap), (false, Some(Gap::NoStore)));
        assert_eq!(
            crate::store::source_presence(&target.join(LOCK)),
            crate::store::SourcePresence::Absent,
            "the lock was taken through the link"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn an_existing_lock_in_a_directory_nothing_can_be_created_in_proves_nothing() {
        let root = scratch("readonly");
        let dir = root.join(DIR);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join(LOCK), "").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let calls = Cell::new(0);
        let creatable = std::fs::File::create(dir.join("probe")).is_ok();
        let out = refresh_with(&root, Path::new(HOME), NOW, asking(&calls, done()));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        if !creatable {
            assert_eq!(calls.get(), 0, "an unpublishable record must not be paid");
            assert_eq!((out.asked, out.gap), (false, Some(Gap::NoStore)));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_absent_binary_asks_nothing_and_records_nothing() {
        let root = scratch("absent");
        let calls = Cell::new(0);
        let out = refresh_with(&root, Path::new(HOME), NOW, asking(&calls, MuseRun::Absent));
        assert_eq!((out.asked, out.gap), (false, Some(Gap::Absent)));
        assert!(matches!(read(&root, Path::new(HOME)), Read::Missing));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_held_lock_is_busy_and_pays_nothing() {
        let root = scratch("busy");
        let dir = root.join(DIR);
        std::fs::create_dir(&dir).unwrap();
        let _guard = crate::store::lock(&dir.join(LOCK), Duration::ZERO).unwrap();
        let calls = Cell::new(0);
        let out = refresh_with(&root, Path::new(HOME), NOW, asking(&calls, done()));
        assert_eq!(calls.get(), 0);
        assert_eq!((out.asked, out.gap), (false, Some(Gap::Busy)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_paid_answer_is_carried_when_only_the_publication_fails() {
        let root = scratch("unrecorded");
        std::fs::create_dir_all(root.join(DIR).join(CACHE)).unwrap();
        let calls = Cell::new(0);
        let out = refresh_with(&root, Path::new(HOME), NOW, asking(&calls, done()));
        assert_eq!(calls.get(), 1);
        assert!(out.asked && out.gap == Some(Gap::Unrecorded));
        assert!(out.answer.is_some_and(|record| record.allowance.is_some()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_failure_class_is_held_across_a_reload_without_vendor_words() {
        let table = [
            (MuseRun::Exited(3), Outcome::Exited(3)),
            (MuseRun::NotUtf8, Outcome::NotUtf8),
            (MuseRun::TooLarge, Outcome::TooLarge),
            (MuseRun::TimedOut, Outcome::TimedOut),
            (MuseRun::Failed, Outcome::Failed),
            (
                MuseRun::Done {
                    text: "not a stream".to_owned(),
                },
                Outcome::Unparsed,
            ),
        ];
        for (index, (run, expected)) in table.into_iter().enumerate() {
            let root = scratch(&format!("class-{index}"));
            let calls = Cell::new(0);
            let first = refresh_with(&root, Path::new(HOME), NOW, asking(&calls, run));
            assert!(first.asked && first.gap.is_none(), "{expected:?}");
            assert_eq!(
                first.answer.as_ref().map(|record| record.outcome),
                Some(expected)
            );
            // Another process: nothing is paid, the class comes back from the file.
            let Read::Cached(held) = read(&root, Path::new(HOME)) else {
                panic!("{expected:?} was not held");
            };
            assert_eq!(held.outcome, expected);
            assert!(held.allowance.is_none());
            let again = refresh_with(&root, Path::new(HOME), NOW + 60, asking(&calls, done()));
            assert!(!again.asked, "{expected:?} paid again inside the hold");
            assert_eq!(calls.get(), 1);
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}
