//! The ENTRY's environment doors — every ambient fact the deleted wrapper used
//! to hand over as a preamble flag.
//!
//! Slice Z1 froze a preamble because bash was in front of the core and knew
//! things the core could not see. Slice Z3 deletes the bash, so there is
//! nothing in front any more and every one of those facts is read HERE, at a
//! named door, with the reason at the site — which is also what
//! `clippy.toml`'s `disallowed-methods` deny requires of any world read.
//!
//! | Door | What it decides | Read in |
//! |---|---|---|
//! | `HOME` | where ae state lives, in both shapes | both |
//! | `PWD` | the caller's working directory | both |
//! | `AE_HOME` | relocates ALL ae state | CHECKOUT only |
//! | `CONFIG_FILE` | which global config is read | CHECKOUT only |
//! | `AE_TMUX_SERVER` + `AE_TMUX_SERVER_KIND` | which tmux server a launch lands on | CHECKOUT only |
//! | `AE_NO_AUTOSTART` | start neither companion | both |
//! | `TMUX` | is this shell in a pane, and which server | both |
//! | `TMUX_PANE` | which pane, for `stop` and `watchdog` | both (at its own site) |

use std::path::{Path, PathBuf};

use crate::inventory::ServerId;
use crate::shape::Shape;

/// `$HOME`, or `None` when it is unset or empty.
#[must_use]
pub fn home() -> Option<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: HOME is where ae state lives — the INSTALLED shape pins $HOME/.ae and the CHECKOUT shape defaults to it"
    )]
    let raw = std::env::var_os("HOME");
    raw.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// The caller's working directory, as the caller spells it.
#[must_use]
pub fn cwd() -> PathBuf {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the working directory is what a nameless launch derives its name from and what a relative config is resolved against"
    )]
    let physical = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: $PWD is the caller's LOGICAL spelling of the same directory — see the note above"
    )]
    let raw = std::env::var_os("PWD");
    let Some(logical) = raw.map(PathBuf::from).filter(|path| path.is_absolute()) else {
        return physical;
    };
    if same_directory(&logical, &physical) {
        logical
    } else {
        physical
    }
}

/// Whether two spellings name the same directory.
fn same_directory(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Where this invocation's state lives — derived, shape-aware.
#[must_use]
pub fn state_root(shape: &Shape) -> Option<PathBuf> {
    if let Some(home) = shape.published_home() {
        return Some(home.to_path_buf());
    }
    ae_home().or_else(|| home().map(|home| home.join(".ae")))
}

/// `AE_HOME` as declared, empty folded to unset. CHECKOUT's door.
#[must_use]
pub fn ae_home() -> Option<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: AE_HOME relocates ALL ae state without swapping $HOME — the ae-dev namespace and the bash suites"
    )]
    let raw = std::env::var_os("AE_HOME");
    raw.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// The global config this invocation reads.
#[must_use]
pub fn config_file(shape: &Shape, root: &Path) -> PathBuf {
    if shape.honours_environment() {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: CONFIG_FILE names the global config a checkout run reads — the orchestrator recipe and the bash suites"
        )]
        let raw = std::env::var_os("CONFIG_FILE");
        if let Some(named) = raw.filter(|value| !value.is_empty()) {
            return PathBuf::from(named);
        }
    }
    root.join("config")
}

/// The project-local config override at `<cwd>/.ae/config`, when there is one.
#[must_use]
pub fn local_config(cwd: &Path) -> Option<PathBuf> {
    let path = cwd.join(".ae").join("config");
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: a project's own .ae/config overrides the global one, and whether it exists is the whole question"
    )]
    let probe = std::fs::metadata(&path);
    probe.is_ok_and(|meta| meta.is_file()).then_some(path)
}

/// `AE_NO_AUTOSTART=1`: start NEITHER companion.
#[must_use]
pub fn no_autostart() -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the operator's AE_NO_AUTOSTART=1 suppresses the watchdog and telegram companions"
    )]
    let raw = std::env::var_os("AE_NO_AUTOSTART");
    raw.is_some_and(|value| value == "1")
}

/// `AE_VERSION` — the target pin, and the ONE word that reads it is
/// [`crate::upgrade`].
#[must_use]
pub fn target_version() -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: AE_VERSION is `ae upgrade`'s target pin and nothing else's input"
    )]
    let raw = std::env::var_os("AE_VERSION");
    raw.filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned())
}

/// `$TMUX`, or `None` when unset or empty.
#[must_use]
pub fn tmux_env() -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: $TMUX is tmux's own marker — the socket path, the client pid and the session id, comma-separated"
    )]
    let raw = std::env::var_os("TMUX");
    raw.filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned())
}

/// `$TMUX_PANE` — the pane THIS process sits in, or `None`.
#[must_use]
pub fn calling_pane_id() -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the pane ae was invoked from is TMUX_PANE, which tmux sets in every pane's environment"
    )]
    let raw = std::env::var_os("TMUX_PANE");
    raw.filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned())
}

/// The `AE_TMUX_SERVER` / `AE_TMUX_SERVER_KIND` pair exactly as declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    /// The kind, VERBATIM when it was set — including `ambiguous`, including
    /// the empty string, and including a word nothing here has heard of.
    pub kind: String,
    /// The value, empty when the variable was unset.
    pub value: String,
}

/// The declared server pair, or `None` when NEITHER variable is set.
#[must_use]
pub fn declared_server(shape: &Shape) -> Option<Declared> {
    if !shape.honours_environment() {
        return None;
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the AE_TMUX_SERVER pair names which tmux server a launch lands on — the ae-dev namespace and the bash suites"
    )]
    let (value, kind) = (
        std::env::var_os("AE_TMUX_SERVER"),
        std::env::var_os("AE_TMUX_SERVER_KIND"),
    );
    if value.is_none() && kind.is_none() {
        return None;
    }
    let value = value.map(|raw| raw.to_string_lossy().into_owned());
    let kind = match kind {
        Some(raw) => raw.to_string_lossy().into_owned(),
        None if value.as_deref().is_some_and(|v| v.starts_with('/')) => "socket".to_owned(),
        None => "name".to_owned(),
    };
    Some(Declared {
        kind,
        value: value.unwrap_or_default(),
    })
}

/// Which server the two PROBES may ask — the wrapper's `tmux` shim, as a value.
#[must_use]
pub fn probe_target(declared: Option<&Declared>) -> Option<ServerId> {
    let Some(declared) = declared else {
        return Some(ServerId::Ambient);
    };
    if declared.kind == "ambiguous" {
        return None;
    }
    ServerId::from_typed_flags(&declared.kind, &declared.value).ok()
}

/// Where a launch ACTUALLY lands, as the typed pair `(kind, value)` — frozen's
/// `resolve_launch_tmux_server`, whole. Both empty means unresolved.
#[must_use]
pub fn resolve_launch_server(declared: Option<&Declared>) -> (String, String) {
    if let Some(server) = probe_target(declared)
        && let Some(socket) = crate::transport::observe_socket_path(&server)
    {
        return prove_socket(&server, socket);
    }
    if let Some(declared) = declared {
        return (declared.kind.clone(), declared.value.clone());
    }
    // DELIBERATELY a bare `$TMUX` read, unlike the client-semantics probe: this
    // is the SOCKET PATH, not our client status. A `$TMUX` inherited by a GUI
    // terminal still names the very server that spawned it, which is the right
    match tmux_env() {
        Some(marker) => (
            "socket".to_owned(),
            marker.split(',').next().unwrap_or_default().to_owned(),
        ),
        None => (String::new(), String::new()),
    }
}

/// Turn a server's own `#{socket_path}` answer into the recorded pair.
fn prove_socket(server: &ServerId, socket: String) -> (String, String) {
    if Path::new(&socket).is_absolute() {
        return ("socket".to_owned(), socket);
    }
    let candidate = std::fs::canonicalize(&socket).ok();
    if let Some(pid) = crate::transport::observe_server_pid(server)
        && let Some(candidate) = candidate
        && let Some(text) = candidate.to_str()
        && crate::transport::observe_server_pid(&ServerId::Selected(crate::meta::Selector::Socket(
            candidate.clone(),
        ))) == Some(pid)
    {
        return ("socket".to_owned(), text.to_owned());
    }
    ("ambiguous".to_owned(), socket)
}

/// Whether this invocation is really inside a tmux pane — frozen's
/// `_ae_inside_tmux`, and NOT a bare `$TMUX` test.
///
/// # Errors
///
/// Propagates a write failure on the caller's stream.
pub fn inside_tmux(
    server: Option<&ServerId>,
    err: &mut impl std::io::Write,
) -> crate::Result<bool> {
    if tmux_env().is_none() {
        return Ok(false);
    }
    let Some(server) = server else {
        return Ok(true);
    };
    let (Some(tty), Some(panes)) = (
        crate::procs::own_tty(),
        crate::transport::observe_pane_ttys(server),
    ) else {
        return Ok(true);
    };
    if panes.is_empty() || crate::tmux::tty_is_a_pane(&tty, &panes) {
        return Ok(true);
    }
    writeln!(
        err,
        "ae: stale $TMUX inherited (this shell is not a tmux pane) — attaching normally."
    )?;
    Ok(false)
}

/// The inherited variables an INSTALLED ae ignores, `NAME=value` each, in a
/// fixed order — the wrapper's one aggregated notice, ported.
#[must_use]
pub fn ignored(shape: &Shape) -> Vec<String> {
    let Some(home) = shape.published_home() else {
        return Vec::new();
    };
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the aggregated notice must name what was SET, so it reads the four variables an installed run then ignores"
    )]
    let read = |name: &str| std::env::var_os(name);
    let mut out = Vec::new();
    let mut note = |name: &str, value: &std::ffi::OsStr| {
        out.push(format!("{name}={}", value.to_string_lossy()));
    };
    if let Some(value) = read("AE_HOME").filter(|value| !same_place(Path::new(value), home)) {
        note("AE_HOME", &value);
    }
    if let Some(value) =
        read("CONFIG_FILE").filter(|value| !same_place(Path::new(value), &home.join("config")))
    {
        note("CONFIG_FILE", &value);
    }
    for name in ["AE_TMUX_SERVER", "AE_TMUX_SERVER_KIND"] {
        if let Some(value) = read(name) {
            note(name, &value);
        }
    }
    out
}

/// Whether two paths name the same PLACE, not merely the same text.
fn same_place(left: &Path, right: &Path) -> bool {
    left == right || resolved_place(left) == resolved_place(right)
}

/// `path` with every link on the way to it resolved.
fn resolved_place(path: &Path) -> PathBuf {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: an aggregated notice about paths has to compare places, not spellings"
    )]
    let whole = std::fs::canonicalize(path);
    if let Ok(whole) = whole {
        return whole;
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return path.to_path_buf();
    };
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the same comparison, one directory up, for a file that does not exist yet"
    )]
    let holder = std::fs::canonicalize(parent);
    holder.map_or_else(|_| path.to_path_buf(), |holder| holder.join(name))
}

/// The one line [`ignored`] is reported with, or `None` when there is nothing
/// to report.
#[must_use]
pub fn notice(ignored: &[String]) -> Option<String> {
    (!ignored.is_empty()).then(|| format!("ae: ignoring inherited {}", ignored.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Selector;

    fn installed() -> Shape {
        Shape::Installed {
            home: PathBuf::from("/u/me/.ae"),
            version_dir: PathBuf::from("/u/me/.ae/versions/1.2.3"),
            version: "1.2.3".to_owned(),
        }
    }

    #[test]
    fn an_installed_run_reads_no_server_pair_at_all() {
        assert_eq!(declared_server(&installed()), None);
        assert_eq!(state_root(&installed()), Some(PathBuf::from("/u/me/.ae")));
        assert_eq!(
            config_file(&installed(), Path::new("/u/me/.ae")),
            PathBuf::from("/u/me/.ae/config")
        );
    }

    #[test]
    fn the_probe_target_is_the_shim_the_wrapper_installed() {
        assert_eq!(probe_target(None), Some(ServerId::Ambient));
        assert_eq!(
            probe_target(Some(&Declared {
                kind: "socket".to_owned(),
                value: "/tmp/s.sock".to_owned(),
            })),
            Some(ServerId::Selected(Selector::Socket(PathBuf::from(
                "/tmp/s.sock"
            ))))
        );
        assert_eq!(
            probe_target(Some(&Declared {
                kind: "name".to_owned(),
                value: "ae-dev".to_owned(),
            })),
            Some(ServerId::Selected(Selector::Name("ae-dev".to_owned())))
        );
        // The refusing shim: never the ambient server.
        for declared in [
            Declared {
                kind: "ambiguous".to_owned(),
                value: "relative.sock".to_owned(),
            },
            Declared {
                kind: "ambiguous".to_owned(),
                value: String::new(),
            },
            Declared {
                kind: "name".to_owned(),
                value: String::new(),
            },
            Declared {
                kind: String::new(),
                value: "/tmp/s.sock".to_owned(),
            },
            Declared {
                kind: "socket-ish".to_owned(),
                value: "/tmp/s.sock".to_owned(),
            },
        ] {
            assert_eq!(probe_target(Some(&declared)), None, "{declared:?}");
        }
    }

    #[test]
    fn an_unroutable_declared_pair_still_crosses_verbatim() {
        // The probe refuses, so nothing is asked and the pair is handed on as
        // it stands — the core, not this module, issues the refusal.
        let declared = Declared {
            kind: "ambiguous".to_owned(),
            value: "relative.sock".to_owned(),
        };
        assert_eq!(
            resolve_launch_server(Some(&declared)),
            ("ambiguous".to_owned(), "relative.sock".to_owned())
        );
    }

    #[test]
    fn an_absolute_socket_answer_is_the_pair() {
        assert_eq!(
            prove_socket(&ServerId::Ambient, "/tmp/tmux-501/ae".to_owned()),
            ("socket".to_owned(), "/tmp/tmux-501/ae".to_owned())
        );
    }

    #[test]
    fn the_notice_names_only_what_would_have_changed_behaviour() {
        assert_eq!(ignored(&Shape::Checkout), Vec::<String>::new());
        assert_eq!(notice(&[]), None);
        assert_eq!(
            notice(&["AE_HOME=/x".to_owned(), "AE_TMUX_SERVER=".to_owned()]),
            Some("ae: ignoring inherited AE_HOME=/x AE_TMUX_SERVER=".to_owned())
        );
    }
}
