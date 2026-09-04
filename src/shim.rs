//! Session helpers as LINKS to the core.
//!
//! Slice Z2 removed the last generated bash from a session directory. A helper
//! is no longer a four-line shim that execs the core; it IS the core, reached
//! through a symlink named `send`, `ask`, `watchdog` and so on. The dispatch
//! that used to be the shim's `exec` line is this module: the core reads the
//! BASENAME of `argv[0]`, and derives the session directory from its dirname.

use std::path::{Component, Path, PathBuf};

/// One session helper: the file name, the core entry it runs, and the words it
/// prepends to the caller's argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Helper {
    /// The file name in the session directory.
    pub name: &'static str,
    /// The core entry the link runs.
    pub entry: &'static str,
    /// Words inserted between the session directory and the caller's argv.
    pub prefix: &'static [&'static str],
}

/// THE helper set — the names a session directory holds, and the only names
/// this dispatch answers to.
pub const HELPERS: [Helper; 21] = [
    Helper {
        name: "send",
        entry: crate::cli::SEND,
        prefix: &[],
    },
    Helper {
        name: "ask",
        entry: crate::cli::ASK,
        prefix: &[],
    },
    Helper {
        name: "review",
        entry: crate::cli::REVIEW,
        prefix: &[],
    },
    Helper {
        name: "reply",
        entry: crate::cli::REPLY,
        prefix: &[],
    },
    Helper {
        name: "requests",
        entry: crate::cli::REQUESTS,
        prefix: &[],
    },
    Helper {
        name: "state",
        entry: crate::cli::STATE,
        prefix: &[],
    },
    Helper {
        name: "mark-done",
        entry: crate::cli::STATE,
        prefix: &["done"],
    },
    Helper {
        name: "say",
        entry: crate::cli::SAY,
        prefix: &[],
    },
    Helper {
        name: "memo",
        entry: crate::cli::MEMO,
        prefix: &[],
    },
    Helper {
        name: "goal",
        entry: crate::cli::GOAL,
        prefix: &[],
    },
    Helper {
        name: "peek",
        entry: crate::cli::PEEK,
        prefix: &[],
    },
    Helper {
        name: "peak",
        entry: crate::cli::PEEK,
        prefix: &[],
    },
    Helper {
        name: "agents",
        entry: crate::cli::AGENTS,
        prefix: &[],
    },
    Helper {
        name: "focus",
        entry: crate::cli::FOCUS,
        prefix: &[],
    },
    Helper {
        name: "interrupt",
        entry: crate::cli::INTERRUPT,
        prefix: &[],
    },
    Helper {
        name: "spawn",
        entry: crate::cli::SPAWN,
        prefix: &[],
    },
    Helper {
        name: "retire",
        entry: crate::cli::RETIRE,
        prefix: &[],
    },
    Helper {
        name: "_register-sid",
        entry: crate::cli::REGISTER_SID,
        prefix: &[],
    },
    Helper {
        name: "watchdog",
        entry: crate::cli::WATCHDOG_RUN,
        prefix: &[],
    },
    Helper {
        name: "loop",
        entry: crate::cli::WATCHDOG_RUN,
        prefix: &[],
    },
    Helper {
        name: "events-tail",
        entry: crate::cli::EVENTS_TAIL,
        prefix: &[],
    },
];

/// The helper `name` names, if any.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static Helper> {
    HELPERS.iter().find(|helper| helper.name == name)
}

/// What `argv[0]` says this process is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// A helper link invoked by path: run `entry` against `dir`.
    Helper {
        /// The helper the basename named.
        helper: &'static Helper,
        /// The session directory, absolute.
        dir: PathBuf,
    },
    /// A helper name with no directory to derive a session from.
    Bare(&'static str),
    /// Not a helper at all — the core under its own name.
    Core,
}

/// Classify `program` (`argv[0]`) against the helper set, resolving a relative
/// directory against `cwd`.
#[must_use]
pub fn classify(program: &str, cwd: &Path) -> Invocation {
    let Some(base) = Path::new(program).file_name().and_then(|n| n.to_str()) else {
        return Invocation::Core;
    };
    let Some(helper) = lookup(base) else {
        return Invocation::Core;
    };
    let parent = Path::new(program).parent().unwrap_or(Path::new(""));
    if parent.as_os_str().is_empty() {
        return Invocation::Bare(helper.name);
    }
    let joined = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        cwd.join(parent)
    };
    Invocation::Helper {
        helper,
        dir: normalise(&joined),
    }
}

/// The one line a helper reached by name is refused with.
#[must_use]
pub fn bare_refusal(name: &str) -> String {
    format!(
        "ae: '{name}' is a session helper — run it by its full path (<session-dir>/{name}); invoked by name it has no session directory to derive."
    )
}

/// Drop `.` components and resolve `..` lexically. No filesystem access.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// The argv the dispatch hands to [`crate::cli::Request::parse`] for `helper`
/// in `dir`, given the words the caller typed after the helper's own name.
#[must_use]
pub fn translate(helper: &Helper, dir: &Path, tail: &[String]) -> Vec<String> {
    let mut argv = Vec::with_capacity(2 + helper.prefix.len() + tail.len());
    argv.push(helper.entry.to_owned());
    argv.push(dir.display().to_string());
    argv.extend(helper.prefix.iter().map(|word| (*word).to_owned()));
    argv.extend_from_slice(tail);
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_invoked_by_path_names_its_own_session() {
        let Invocation::Helper { helper, dir } = classify("/s/tg1/send", Path::new("/anywhere"))
        else {
            panic!("a path invocation is a helper");
        };
        assert_eq!(helper.entry, crate::cli::SEND);
        assert_eq!(dir, Path::new("/s/tg1"));
    }

    #[test]
    fn a_relative_link_resolves_against_the_working_directory() {
        let Invocation::Helper { dir, .. } = classify("./peek", Path::new("/s/tg1")) else {
            panic!("a relative path invocation is a helper");
        };
        assert_eq!(dir, Path::new("/s/tg1"));
    }

    #[test]
    fn a_bare_helper_name_is_refused_by_name() {
        assert_eq!(classify("send", Path::new("/s")), Invocation::Bare("send"));
        assert!(bare_refusal("send").contains("<session-dir>/send"));
    }

    #[test]
    fn the_core_under_its_own_name_is_not_a_helper() {
        assert_eq!(
            classify("/opt/ae/ae-core", Path::new("/s")),
            Invocation::Core
        );
        assert_eq!(classify("ae", Path::new("/s")), Invocation::Core);
    }

    #[test]
    fn mark_done_is_state_done_with_the_rest_as_the_reason() {
        let helper = lookup("mark-done").expect("mark-done is a helper");
        let argv = translate(helper, Path::new("/s/tg1"), &["shipped".to_owned()]);
        assert_eq!(argv, ["_state", "/s/tg1", "done", "shipped"]);
    }

    #[test]
    fn the_aliases_reach_the_same_entries_as_the_names_they_alias() {
        for (alias, name) in [("peak", "peek"), ("loop", "watchdog")] {
            let (a, n) = (lookup(alias).expect(alias), lookup(name).expect(name));
            assert_eq!(a.entry, n.entry, "{alias} aliases {name}");
            assert!(a.prefix.is_empty());
        }
    }

    #[test]
    fn every_helper_name_is_distinct() {
        let mut names: Vec<&str> = HELPERS.iter().map(|helper| helper.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(
            names.len(),
            count,
            "a duplicate name is a link written twice"
        );
    }
}
