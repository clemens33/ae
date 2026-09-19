//! Session helpers as LINKS to the core.
//!
//! A helper IS the core, reached through a symlink named `send`, `ask`,
//! `watchdog` and so on. The dispatch is this module: the core reads the
//! BASENAME of `argv[0]`, and derives the session directory from its dirname.
//!
//! A helper has TWO accepted spellings and this module owns both, because a
//! second helper table is a second set of names to drift. The LINK
//! (`<state-root>/sessions/<session>/send lead "hi"`) names its session by the
//! path it was invoked through; the SHORT FORM (`ae @<session> send lead "hi"`)
//! names it with a marked first word to the core's own argv. Both end at the
//! same [`Helper`], the same [`translate`] and the same dispatch, and a helper
//! reached by BARE name is still refused by [`bare_refusal`] — a name with no
//! `/` and no `@` has no session to derive and ae will not guess one.
//!
//! What the short form does NOT carry is AUTHORITY. It selects the session
//! directory a helper acts on, nothing else: caller identity stays whatever the
//! live pane proves, exactly as it does through the link.

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
pub const HELPERS: [Helper; 25] = [
    Helper {
        name: "send",
        entry: crate::cli::SEND,
        prefix: &[],
    },
    Helper {
        name: "relay",
        entry: crate::cli::RELAY,
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
        name: "quota",
        entry: crate::cli::QUOTA,
        prefix: &[],
    },
    Helper {
        name: "usage",
        entry: crate::cli::USAGE,
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
        name: "relaunch",
        entry: crate::cli::RELAUNCH,
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
///
/// It names BOTH accepted spellings, because the refusal is the only place a
/// caller who typed the third one is reading.
#[must_use]
pub fn bare_refusal(name: &str) -> String {
    format!(
        "ae: '{name}' is a session helper — run it by its full path (<session-dir>/{name}) or as 'ae @<session> {name} …'; invoked by name it has no session directory to derive."
    )
}

/// The marker that makes a public `ae` argv a helper call: `ae @<session>
/// <helper> <args…>`. Attached to the session, so `ae @ send` is not it.
pub const SESSION_MARKER: char = '@';

/// The accepted spellings, printed under every refusal this route writes.
///
/// It names the link too: the short form is a second spelling, never a
/// replacement, and a caller who got the marker wrong may well want the path.
pub const SHORT_FORM_USAGE: &str = "Usage: ae @<session> <helper> [args…]   (example: ae @demo send lead 'ready')\n       the session's own link stays valid: <state-root>/sessions/<session>/<helper> [args…]";

/// What a public argv says about the short helper form.
///
/// Every arm is decided from the WORDS alone — the session still has to be
/// proven on disk by the caller, which is where the marker's one privilege
/// ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Short<'a> {
    /// No leading marker: this argv is not the short form and the ordinary
    /// route still owns it.
    Absent,
    /// A well-formed call, parsed and nothing more.
    Call {
        /// The session named after the marker, already through the grammar.
        session: &'a str,
        /// The helper the second word named.
        helper: &'static Helper,
        /// Every word after the helper, untouched.
        tail: &'a [String],
    },
    /// The marker was typed and the rest was not a helper call: the refusal to
    /// print, exit 2, with nothing on disk read and nothing written.
    Usage(String),
}

/// Classify a PUBLIC argv (the words after the program name) against the short
/// helper form.
///
/// The marker is parsed here, above every fall-through, so `@…` can never reach
/// the launch route and create, resume or rename a session.
#[must_use]
pub fn short_form(args: &[String]) -> Short<'_> {
    let Some(session) = args
        .first()
        .and_then(|word| word.strip_prefix(SESSION_MARKER))
    else {
        return Short::Absent;
    };
    if session.is_empty() {
        return Short::Usage(short_refusal("'@' names no session"));
    }
    // The canonical session-name grammar is also the traversal guard: it admits
    // no `/`, no `.` and no `..`, so a marked word can never leave the sessions
    // root.
    if !crate::session_launch::name::is_session_name(session) {
        return Short::Usage(short_refusal(&format!("'{session}' is not a session name")));
    }
    let Some(word) = args.get(1) else {
        return Short::Usage(short_refusal(&format!("'@{session}' names no helper")));
    };
    let Some(helper) = lookup(word) else {
        return Short::Usage(short_refusal(&format!("'{word}' is not a session helper")));
    };
    Short::Call {
        session,
        helper,
        tail: &args[2..],
    }
}

/// One refusal: what was wrong, then the spellings that are right.
fn short_refusal(why: &str) -> String {
    format!("ae: {why}.\n{SHORT_FORM_USAGE}\n")
}

/// Drop `.` components and resolve `..` lexically.
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
    fn relay_is_a_universal_link_to_its_privileged_core_entry() {
        let helper = lookup("relay").expect("relay is a helper");
        let argv = translate(
            helper,
            Path::new("/s/orchestrator"),
            &["work:lead".to_owned(), "ship it".to_owned()],
        );
        assert_eq!(argv, ["_relay", "/s/orchestrator", "work:lead", "ship it"]);
    }

    #[test]
    fn quota_is_a_session_derived_read_only_helper() {
        let helper = lookup("quota").expect("quota is a helper");
        let argv = translate(helper, Path::new("/s/work"), &[]);
        assert_eq!(argv, ["_quota", "/s/work"]);
    }

    #[test]
    fn usage_is_a_session_derived_read_only_helper() {
        let helper = lookup("usage").expect("usage is a helper");
        let argv = translate(helper, Path::new("/s/work"), &[]);
        assert_eq!(argv, ["_usage", "/s/work"]);
        assert_eq!(HELPERS.len(), 25);
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
    fn the_short_form_takes_every_helper_name_and_no_other_word() {
        for helper in HELPERS {
            let argv = [
                "@demo".to_owned(),
                helper.name.to_owned(),
                "tail".to_owned(),
            ];
            let Short::Call {
                session,
                helper: taken,
                tail,
            } = short_form(&argv)
            else {
                panic!("'{}' is a helper, so the short form takes it", helper.name);
            };
            assert_eq!(session, "demo");
            assert_eq!(*taken, helper, "the one table answered");
            assert_eq!(tail, ["tail"], "the tail is untouched");
        }
        // A word the table does not hold is a usage error, never a core
        // command and never a session name.
        for word in ["list", "brief", "_launch", "sned", ""] {
            let argv = ["@demo".to_owned(), word.to_owned()];
            assert!(
                matches!(short_form(&argv), Short::Usage(_)),
                "'{word}' is not a helper"
            );
        }
    }

    #[test]
    fn the_short_form_answers_only_a_marked_first_word() {
        for argv in [vec![], vec!["send"], vec!["demo", "send"], vec!["list"]] {
            let argv: Vec<String> = argv.into_iter().map(str::to_owned).collect();
            assert_eq!(
                short_form(&argv),
                Short::Absent,
                "{argv:?} carries no marker, so the ordinary route still owns it"
            );
        }
    }

    #[test]
    fn a_marked_word_that_is_not_a_helper_call_is_a_usage_error_naming_the_spelling() {
        for argv in [
            vec!["@"],
            vec!["@", "send"],
            vec!["@demo"],
            vec!["@demo", "nosuch"],
            vec!["@..", "send"],
            vec!["@../escape", "send"],
            vec!["@a/b", "send"],
            vec!["@-lead", "send"],
            vec!["@.hidden", "send"],
        ] {
            let owned: Vec<String> = argv.iter().map(|word| (*word).to_owned()).collect();
            let Short::Usage(refusal) = short_form(&owned) else {
                panic!("{argv:?} is not a helper call");
            };
            assert!(
                refusal.contains("ae @<session> <helper>"),
                "{argv:?} must name the accepted spelling: {refusal}"
            );
            assert!(
                refusal.contains("the session's own link stays valid"),
                "{argv:?} must keep the path spelling on offer: {refusal}"
            );
        }
    }

    #[test]
    fn the_short_form_and_the_link_reach_one_translation() {
        // The fixed prefix is the proof: `mark-done` is `state done`, and the
        // short form gets it from the same table rather than a second copy.
        let argv = [
            "@demo".to_owned(),
            "mark-done".to_owned(),
            "shipped".to_owned(),
        ];
        let Short::Call { helper, tail, .. } = short_form(&argv) else {
            panic!("mark-done is a helper");
        };
        assert_eq!(
            translate(helper, Path::new("/s/demo"), tail),
            ["_state", "/s/demo", "done", "shipped"]
        );
        // And an alias lands on the entry it aliases, through the same route.
        for (alias, name) in [("peak", "peek"), ("loop", "watchdog")] {
            let aliased = ["@demo".to_owned(), alias.to_owned()];
            let named = ["@demo".to_owned(), name.to_owned()];
            let (Short::Call { helper: a, .. }, Short::Call { helper: n, .. }) =
                (short_form(&aliased), short_form(&named))
            else {
                panic!("{alias} and {name} are both helpers");
            };
            assert_eq!(a.entry, n.entry, "{alias} aliases {name}");
        }
    }

    #[test]
    fn the_bare_refusal_names_both_accepted_spellings() {
        let refusal = bare_refusal("send");
        assert!(refusal.contains("<session-dir>/send"), "{refusal}");
        assert!(refusal.contains("ae @<session> send"), "{refusal}");
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
