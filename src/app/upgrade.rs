//! A running `ae app` that finds another core installed: the command link
//! names a version this core is not. Once nothing in it would be lost, the
//! app in its session's chat window has tmux respawn its pane, and an app in
//! no chat window replaces its own process with the linked core, so the shell
//! that started it stays. A chat window whose proof fails only says so on its
//! hint row.
//!
//! The pure halves live here; the reader asks [`seen`] with each fleet read,
//! and the loop asks [`restart_blocker`] on every tick.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::handover::{self, View};
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
    pub restart: Result<Via, String>,
}

/// How an app moves to another core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Via {
    /// tmux respawns the app's pane: the app proven its session's chat window.
    Respawn(Restart),
    /// The app replaces its own process with the command link: any app in no
    /// chat window.
    Exec { link: PathBuf },
}

impl From<Restart> for Via {
    fn from(restart: Restart) -> Self {
        Self::Respawn(restart)
    }
}

/// Which way an app moves, by its own pane's `@ae_console` stamp: `None` for
/// an app in no tmux pane, `Some(None)` when tmux did not answer. A stamped
/// pane is a chat window and moves only by `chat`'s proven respawn: its refusal
/// stays a refusal, never an exec. An unstamped pane, or none, execs `link`.
///
/// # Errors
///
/// Why this app only says so.
pub fn via(
    stamp: Option<Option<&str>>,
    chat: impl FnOnce() -> Result<Restart, String>,
    link: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<Via, String> {
    match stamp {
        Some(None) => Err("tmux did not answer this pane's stamp".to_owned()),
        Some(Some(stamp)) if !stamp.is_empty() => chat().map(Via::Respawn),
        _ => link().map(|link| Via::Exec { link }),
    }
}

/// What an exec restart runs: the link, `app`, the human's `operands` as typed,
/// then the handed-over `view` behind [`handover::FLAG`], or [`super::OWN`]
/// when there is none — only when `target` is newer than `own`, because an
/// older core may not know either flag. A rollback starts with a fresh view.
#[must_use]
pub fn exec_argv(
    link: &Path,
    operands: &[String],
    view: Option<String>,
    target: &str,
    own: &str,
) -> Vec<String> {
    let newer = crate::install::compare_versions(target, own) == Some(Ordering::Greater);
    let carrier = match view.filter(|_| newer) {
        Some(text) => vec![handover::FLAG.to_owned(), text],
        None if newer => vec![super::OWN.to_owned()],
        None => Vec::new(),
    };
    [link.display().to_string(), "app".to_owned()]
        .into_iter()
        .chain(operands.iter().cloned())
        .chain(carrier)
        .collect()
}

/// What the view an exec restart handed this app says to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Carried {
    /// The view to adopt, and the instant it was written.
    View(View, i64),
    /// For this process and fresh, but for this version: the exec landed in a
    /// core the link did not name, so this app never restarts again.
    Landed(String),
    /// Anything else: the default view, and nothing refused.
    Ignored,
}

/// What `text` hands the app of process `pid` running version `own` at `now`:
/// only a canonical handover written for `%<pid>` — exec keeps the pid — and
/// still fresh counts.
#[must_use]
pub fn carried(text: &str, pid: u32, own: &str, now: i64) -> Carried {
    let Some(held) = handover::parse(text) else {
        return Carried::Ignored;
    };
    if held.pane != format!("%{pid}") || !handover::fresh(held.at, now) {
        Carried::Ignored
    } else if held.version == own {
        Carried::View(held.view, held.at)
    } else {
        Carried::Landed(held.version)
    }
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
    /// The settings overlay or the peek panel is open.
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
    prove: impl FnOnce() -> Result<Via, String>,
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

/// The command link an exec restart runs, spelled from `home` exactly as
/// [`link_version`] reads it: the link exec'd is the link compared. Only a
/// UTF-8 spelling, since argv is text.
pub(super) fn link_path(home: Option<&Path>) -> Result<PathBuf, String> {
    let home = home.ok_or("HOME is not set")?;
    let link = crate::install::fixed_paths(home)?.link;
    link.to_str().ok_or("the command link is not UTF-8")?;
    Ok(link)
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

/// `upgrade` as this app may act on it: never an exec into `unlinked`, the
/// version the link already named at its first look — this core did not
/// arrive through that link, so an exec of it would only land here again. A
/// chat window's proven respawn is left as it is.
#[must_use]
pub fn unlinked(mut upgrade: Upgrade, unlinked: Option<&str>) -> Upgrade {
    if unlinked == Some(upgrade.version.as_str()) && matches!(upgrade.restart, Ok(Via::Exec { .. }))
    {
        let version = &upgrade.version;
        upgrade.restart = Err(format!(
            "this app did not start from the link to ae {version}"
        ));
    }
    upgrade
}

/// The hint row of an app whose exec into `version` failed: it keeps running
/// and does not try that version again.
#[must_use]
pub fn failed_hint(version: &str, why: &str) -> String {
    crate::board::terminal_text(&format!("could not restart into ae {version} - {why}"))
}

/// What the hint row says of `upgrade`, given the exec that `failed`, if any:
/// the version it tried, and why.
pub(super) fn hint(upgrade: &Upgrade, failed: Option<&(String, String)>) -> String {
    match (&upgrade.restart, failed) {
        (_, Some((version, why))) if *version == upgrade.version => failed_hint(version, why),
        (Ok(_), _) => busy_hint(&upgrade.version),
        (Err(_), _) => hint_only(&upgrade.version),
    }
}

#[cfg(test)]
mod tests {
    use super::{Carried, Restart, Upgrade, Via, carried, exec_argv, link_version, unlinked, via};
    use crate::app::handover::{Handover, View};
    use crate::inventory::ServerId;
    use std::path::{Path, PathBuf};

    /// D1: only a stamped pane is a chat window, and it moves only by its
    /// proven respawn — a refused proof stays a refusal; an unstamped pane or
    /// none execs the link; a pane tmux did not answer for does nothing.
    #[test]
    fn a_chat_window_never_execs_and_any_other_app_does() {
        let restart = Restart {
            server: ServerId::Ambient,
            pane: "%3".to_owned(),
            command: vec!["ae".to_owned()],
        };
        let link = PathBuf::from("/h/.local/bin/ae");
        let exec = Ok(Via::Exec { link: link.clone() });
        let proven = || Ok(restart.clone());
        let refused = || Err("this pane is not the session's chat window".to_owned());
        let linked = || Ok(link.clone());
        assert_eq!(via(None, refused, linked), exec, "no tmux pane");
        assert_eq!(via(Some(Some("")), refused, linked), exec, "unstamped");
        assert_eq!(
            via(Some(Some("u")), proven, linked),
            Ok(Via::Respawn(restart.clone()))
        );
        let chat = via(Some(Some("u")), refused, linked);
        let left = Err("this pane is not the session's chat window".to_owned());
        assert_eq!(chat, left, "a refused chat never execs");
        assert!(via(Some(None), proven, linked).is_err(), "no answer");
        assert!(via(None, proven, || Err("HOME is not set".to_owned())).is_err());
        // The version the link named at the first look: only its exec is withheld.
        let up = |restart| Upgrade {
            version: "2026.10.99".to_owned(),
            restart,
        };
        assert!(
            unlinked(up(exec.clone()), Some("2026.10.99"))
                .restart
                .is_err()
        );
        for first in [Some("2026.10.98"), None] {
            assert_eq!(unlinked(up(exec.clone()), first).restart, exec);
        }
        let respawn = Ok(Via::Respawn(restart));
        assert_eq!(
            unlinked(up(respawn.clone()), Some("2026.10.99")).restart,
            respawn
        );
    }

    /// The exec argv: the link, `app`, the operands as typed, and the view
    /// only for a newer target — an older one, or none, starts fresh. A newer
    /// target with no view is told the start is ae's own instead.
    #[test]
    fn the_exec_hands_on_the_operands_and_the_view_only_to_a_newer_core() {
        let link = Path::new("/h/.local/bin/ae");
        let typed = ["web".to_owned()];
        let view = || Some("v1|%7|x".to_owned());
        let newer = exec_argv(link, &typed, view(), "2026.10.99", "2026.10.59");
        let want = ["/h/.local/bin/ae", "app", "web", "--app-view", "v1|%7|x"];
        assert_eq!(newer, want);
        let older = exec_argv(link, &typed, view(), "2026.10.40", "2026.10.59");
        assert_eq!(older, ["/h/.local/bin/ae", "app", "web"]);
        let bare = exec_argv(link, &[], None, "2026.10.99", "2026.10.59");
        assert_eq!(bare, ["/h/.local/bin/ae", "app", "--app-own"]);
        let plain = exec_argv(link, &typed, None, "2026.10.40", "2026.10.59");
        assert_eq!(plain, ["/h/.local/bin/ae", "app", "web"], "an older core");
        let unparsed = exec_argv(link, &[], view(), "next", "2026.10.59");
        assert_eq!(unparsed, ["/h/.local/bin/ae", "app"], "no order, no view");
    }

    /// G2 and the hostile carrier: only a canonical view for this pid, fresh
    /// and for this version, is adopted; for this pid and fresh but another
    /// version it has landed elsewhere; anything else is ignored.
    #[test]
    fn only_a_fresh_view_for_this_process_and_version_is_adopted() {
        let view = View {
            session: "web".to_owned(),
            id: "6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11".to_owned(),
            focus: None,
            sidebar: None,
            list: None,
            pages: 2,
            rows: 0,
            list_top: None,
            body_top: 4,
        };
        let text = |pane: &str, version: &str, at: i64| {
            let (pane, version) = (pane.to_owned(), version.to_owned());
            let view = view.clone();
            Handover {
                pane,
                version,
                at,
                view,
            }
            .render()
        };
        let own = "2026.10.99";
        let adopt = carried(&text("%42", own, 100), 42, own, 130);
        assert_eq!(adopt, Carried::View(view.clone(), 100));
        let landed = carried(&text("%42", own, 100), 42, "2026.10.59", 100);
        assert_eq!(landed, Carried::Landed(own.to_owned()));
        for (written, now) in [(text("%43", own, 100), 100), (text("%42", own, 100), 161)] {
            assert_eq!(carried(&written, 42, own, now), Carried::Ignored);
        }
        let future = carried(&text("%42", "2026.10.59", 101), 42, own, 100);
        assert_eq!(future, Carried::Ignored, "from the future, even elsewhere");
        for garbage in [
            "",
            "hostile",
            "v1|%42",
            &format!("{} ", text("%42", own, 100)),
        ] {
            assert_eq!(carried(garbage, 42, own, 100), Carried::Ignored);
        }
    }

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
