//! A running `ae app` that finds another core installed: the command link
//! names a version this core is not. The app in its session's chat window
//! restarts itself into it once nothing in it would be lost; any other app
//! only says so on its hint row.
//!
//! The pure halves live here; the reader asks [`seen`] with each fleet read,
//! and the loop asks [`restart_blocker`] on every tick.

use std::cmp::Ordering;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::console::toggle::Restart;
use crate::shape::Shape;

/// How long after the last key, click or paste a restart waits.
const QUIET: Duration = super::REFRESH;

/// Another installed core, and how this app moves to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgrade {
    /// The version the command link names.
    pub version: String,
    /// The restart into it, or why this app only says so.
    pub restart: Result<Restart, String>,
}

/// What holds a restart back. Each is something a restart would lose or
/// a human in the middle of a gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    /// A draft in any composer, kept or being written.
    Draft,
    /// An ask entered but not delivered yet.
    Queued,
    /// Keys held behind a frame that has not been drawn.
    Deferred,
    /// The app is writing: its next keys are draft.
    Composing,
    /// An entry was refused and its keys are swallowed.
    Held,
    /// The settings overlay is open.
    Settings,
    /// A turn is marked for copying.
    Turn,
    /// A key waits for its second press.
    Armed,
    /// A key, click or paste came in the last [`QUIET`].
    RecentInput,
}

/// What the command link says to an app of `shape`: `None` unless the shape
/// is installed and `linked` — given the ae home — names a valid version
/// other than this one, newer or older. The restart is `prove`n only while
/// [`crate::autoupgrade::automatic`] says this core acts on its own.
///
/// This core's version is the shape's: the install gate proved it is the
/// version this binary reports.
#[must_use]
pub fn seen(
    shape: &Shape,
    linked: impl FnOnce(&Path) -> Result<String, String>,
    automatic: impl FnOnce() -> bool,
    prove: impl FnOnce() -> Result<Restart, String>,
) -> Option<Upgrade> {
    let Shape::Installed { home, version, .. } = shape else {
        return None;
    };
    let linked = linked(home).ok()?;
    if crate::install::compare_versions(&linked, version)? == Ordering::Equal {
        return None;
    }
    let restart = if automatic() {
        prove()
    } else {
        Err("auto_upgrade is off".to_owned())
    };
    Some(Upgrade {
        version: linked,
        restart,
    })
}

/// The version the command link names, the link spelled from `home` — the
/// `$HOME` a publish spells it from: the shape's home is canonical, and the
/// link's target is judged by its spelling.
pub(super) fn link_version(home: Option<&Path>) -> Result<String, String> {
    let home = home.ok_or("HOME is not set")?;
    crate::install::current_public_version(&crate::install::fixed_paths(home)?)
}

/// The first thing that holds a restart back, in the order of [`Blocker`],
/// or `None` when the app is idle. Input at `last_input` holds it for
/// [`QUIET`] after.
#[must_use]
#[allow(
    clippy::too_many_arguments,
    clippy::fn_params_excessive_bools,
    reason = "the idle gate's table, one flag per row, as the acceptance spec pins it"
)]
pub fn restart_blocker(
    draft: bool,
    queued: bool,
    deferred: bool,
    composing: bool,
    held: bool,
    settings: bool,
    turn: bool,
    armed: bool,
    last_input: Option<Instant>,
    now: Instant,
) -> Option<Blocker> {
    let recent = last_input.is_some_and(|at| now.saturating_duration_since(at) < QUIET);
    [
        (draft, Blocker::Draft),
        (queued, Blocker::Queued),
        (deferred, Blocker::Deferred),
        (composing, Blocker::Composing),
        (held, Blocker::Held),
        (settings, Blocker::Settings),
        (turn, Blocker::Turn),
        (armed, Blocker::Armed),
        (recent, Blocker::RecentInput),
    ]
    .into_iter()
    .find_map(|(on, blocker)| on.then_some(blocker))
}

/// The hint row while a restart waits for the app to be idle.
#[must_use]
pub fn busy_hint(version: &str) -> String {
    format!("ae {version} installed - restarts when idle")
}

/// The hint row of an app that does not restart itself.
#[must_use]
pub fn hint_only(version: &str) -> String {
    format!("ae {version} installed - restart ae app to use it")
}

/// What the hint row says of `upgrade`.
pub(super) fn hint(upgrade: &Upgrade) -> String {
    match upgrade.restart {
        Ok(_) => busy_hint(&upgrade.version),
        Err(_) => hint_only(&upgrade.version),
    }
}

#[cfg(test)]
mod tests {
    use super::link_version;

    /// U1: the link is read in `$HOME`'s spelling, as the publish wrote it;
    /// another spelling of the same directory names no published version.
    #[test]
    fn the_link_is_read_in_the_spelling_home_gives_it() {
        let root = std::env::temp_dir().join(format!("ae-app-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (real, alias) = (root.join("real"), root.join("alias"));
        std::fs::create_dir_all(real.join(".local/bin")).expect("bin");
        std::fs::create_dir_all(real.join(".ae/versions/2026.10.99")).expect("version");
        std::os::unix::fs::symlink(&real, &alias).expect("alias");
        let core = alias.join(".ae/versions/2026.10.99/ae-core");
        std::os::unix::fs::symlink(core, real.join(".local/bin/ae")).expect("link");
        assert_eq!(link_version(Some(&alias)).as_deref(), Ok("2026.10.99"));
        assert!(link_version(Some(&real)).is_err(), "another spelling");
        assert!(link_version(None).is_err(), "no HOME");
        let _ = std::fs::remove_dir_all(&root);
    }
}
