//! Argv parsing: the one place that turns a command line into an intent.
//!
//! Deliberately hand-rolled. A CLI argument parser is a dependency the skeleton
//! does not need, and #80's rule for the error crate — "no dependency exists
//! until a real error does" — applies here too. When the real command surface
//! arrives (P1+), revisit with a measured need.

use std::path::PathBuf;

use crate::filters::{ListArgs, UnknownFlag};
use crate::requests::Mode;

/// The subcommand that carries the `requests` helper surface.
///
/// **The leading underscore is the whole argument for this spelling** (lead
/// ruling on D1, 2026-08-24). `_validate_session_name` forbids a leading `_`, so
/// an underscore-prefixed subcommand can never shadow a legal session name and
/// SC-022's "a top-level bare word is a launch candidate" loses nothing to it.
/// A `requests` spelling would have taken a real name out of the launch grammar;
/// a `--session` flag would have invented an option no row names. `ae` already
/// spells its internal surfaces this way — `_register-sid`, `_recover-pending`.
pub const REQUESTS: &str = "_requests";

/// The subcommand that carries the `events-tail` helper surface. Same
/// underscore reasoning as [`REQUESTS`].
pub const EVENTS_TAIL: &str = "_events-tail";

/// The `state` helper's write surface — `_state <meta-dir> <value> [reason…]`.
/// Underscored like [`REQUESTS`]: launched by the generated helper, not typed.
pub const STATE: &str = "_state";

/// What an argv asks the binary to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Print the version line.
    Version,
    /// Print usage.
    Help,
    /// `list` (or its `ls` spelling) with the filters its flags selected.
    List(ListArgs),
    /// `_requests <meta-dir> [mine|inbox|all]` — the `requests` helper surface.
    Requests {
        /// The session meta directory the frozen helper derives from `$0`.
        dir: PathBuf,
        /// SC-212c's mode, defaulting to `mine`.
        mode: Mode,
    },
    /// `_events-tail <meta-dir>` — the `events-tail` helper surface.
    /// `_state <meta-dir> <value> [reason…]` — the `state` helper's write path.
    /// The tail is validated by [`crate::state::parse`], not here: the usage
    /// text is the helper's own and belongs beside the rule it states.
    State {
        /// The session meta directory.
        dir: std::path::PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    EventsTail {
        /// The session meta directory the frozen helper derives from `$0`.
        dir: PathBuf,
    },
    /// A usage error about a MISSING operand rather than an offending token.
    ///
    /// The successor spelling's own error class: the frozen helpers read their
    /// meta directory from `$0`, so "you gave me no directory" is a state they
    /// cannot be in and no row rules. It exits `2` like every other usage error,
    /// and it is a separate variant because [`Request::UsageError`] promises to
    /// carry the offending token verbatim — and an absence is not a token.
    MissingOperand(&'static str),
    /// **SC-022** — a token that is a usage error: an unknown top-level OPTION,
    /// or an unknown token in a `list`/`ls` tail. Carries the token verbatim.
    UsageError(String),
    /// A top-level NON-option token: a session name under the start grammar.
    ///
    /// **SC-022 rules this is NEVER an unknown-subcommand error.** It is a
    /// launch candidate, and launching is not this slice's work — so the
    /// variant exists to keep the misclassification impossible rather than to
    /// implement anything. There is deliberately no "unknown command" phrase in
    /// this crate for such a token to fall into.
    LaunchCandidate(String),
}

impl Request {
    /// Classify `args` — argv WITHOUT the program name.
    ///
    /// An empty argv is [`Request::Help`]: a multiplexer that prints nothing
    /// when invoked bare is a multiplexer nobody can discover.
    ///
    /// # `list` / `ls`
    ///
    /// The flag grammar is **not** repeated here — it is
    /// [`ListArgs::parse`](crate::filters::ListArgs::parse), which owns
    /// SC-017a–f, SC-017i and SC-521a/b. This function only decides that the
    /// word `list` (or `ls`) hands the REST of argv to that parser, so there is
    /// exactly one place where a flag means something.
    ///
    /// **SC-021** — `ls` is an alias of `list`, so the two are one command with
    /// two spellings rather than two commands that happen to agree. The row's
    /// authority is the S1 surface INVENTORY, not commands.md, where `ls`
    /// appears nowhere: the row records that documentation gap as its own
    /// finding.
    ///
    /// # SC-022 — the two kinds of unrecognised token
    ///
    /// A token `list` does not know is a [`Request::UsageError`]. So is an
    /// unknown top-level OPTION — a `-`/`--` token the dispatcher does not
    /// define. A top-level token that is NOT option-shaped is neither: it is a
    /// session name under the start grammar, and becomes a
    /// [`Request::LaunchCandidate`]. The row rules that direction explicitly, so
    /// the shape of this function is the shape of the row.
    ///
    /// ```
    /// use ae::cli::Request;
    /// use ae::filters::{ListArgs, Scope};
    ///
    /// assert_eq!(Request::parse(&[]), Request::Help);
    /// assert_eq!(Request::parse(&["-V".to_owned()]), Request::Version);
    ///
    /// let Request::List(args) = Request::parse(&["ls".to_owned(), "--all".to_owned()]) else {
    ///     panic!("ls is the list command");
    /// };
    /// assert_eq!(args.selection.scope, Scope::All);
    /// assert_eq!(Request::parse(&["list".to_owned()]), Request::List(ListArgs::default()));
    ///
    /// // SC-022: option-shaped is a usage error; a bare word is a session name.
    /// assert_eq!(
    ///     Request::parse(&["--frobnicate".to_owned()]),
    ///     Request::UsageError("--frobnicate".to_owned())
    /// );
    /// assert_eq!(
    ///     Request::parse(&["my-feature".to_owned()]),
    ///     Request::LaunchCandidate("my-feature".to_owned())
    /// );
    /// ```
    ///
    /// # The internal helper surfaces: `_requests`, `_events-tail`, `_state`
    ///
    /// ```
    /// use ae::cli::Request;
    /// use ae::requests::Mode;
    ///
    /// let argv = ["_requests".to_owned(), "/s/tg1".to_owned(), "all".to_owned()];
    /// assert_eq!(
    ///     Request::parse(&argv),
    ///     Request::Requests { dir: "/s/tg1".into(), mode: Mode::All }
    /// );
    /// // The mode defaults, exactly as the helper's `${1:-mine}` does.
    /// let argv = ["_requests".to_owned(), "/s/tg1".to_owned()];
    /// assert_eq!(
    ///     Request::parse(&argv),
    ///     Request::Requests { dir: "/s/tg1".into(), mode: Mode::Mine }
    /// );
    /// ```
    #[must_use]
    pub fn parse(args: &[String]) -> Self {
        match args.first().map(String::as_str) {
            None | Some("-h" | "--help" | "help") => Self::Help,
            Some("-V" | "--version" | "version") => Self::Version,
            Some("list" | "ls") => match ListArgs::parse(&args[1..]) {
                Ok(parsed) => Self::List(parsed),
                Err(UnknownFlag(token)) => Self::UsageError(token),
            },
            Some(REQUESTS) => Self::parse_requests(&args[1..]),
            Some(STATE) => match &args[1..] {
                [] => Self::MissingOperand(STATE),
                [dir, rest @ ..] => Self::State {
                    dir: dir.into(),
                    tail: rest.to_vec(),
                },
            },
            Some(EVENTS_TAIL) => match &args[1..] {
                [] => Self::MissingOperand(EVENTS_TAIL),
                [dir] => Self::EventsTail { dir: dir.into() },
                [_, extra, ..] => Self::UsageError(extra.clone()),
            },
            // SC-022, in the order the row states it: option-shaped first,
            // because everything left over is a name and not an error.
            Some(other) if other.starts_with('-') => Self::UsageError(other.to_owned()),
            Some(other) => Self::LaunchCandidate(other.to_owned()),
        }
    }

    /// `_requests <meta-dir> [mine|inbox|all]`.
    ///
    /// A mode token that is not one of the three is a usage error at `2`, not
    /// the frozen helper's `1` (lead ruling on D2): no corpus row exercises this
    /// path, and where the corpus pins nothing and the crate's exit-code
    /// contract speaks, the contract wins — `1` for both conflates "you asked
    /// wrong" with "it went wrong", which is the distinction `2` exists for.
    /// The IDENTITY refusal keeps its pinned `1`; that one is
    /// [`crate::requests::EXIT_NO_IDENTITY`] and is not decided from argv.
    fn parse_requests(tail: &[String]) -> Self {
        let (dir, token) = match tail {
            [] => return Self::MissingOperand(REQUESTS),
            [dir] => (dir, None),
            [dir, mode] => (dir, Some(mode.as_str())),
            [_, _, extra, ..] => return Self::UsageError(extra.clone()),
        };
        match Mode::parse(token) {
            Some(mode) => Self::Requests {
                dir: dir.into(),
                mode,
            },
            // `token` is necessarily `Some` on this arm: `Mode::parse(None)` is
            // the documented `mine` default and cannot fail. The fallback is
            // written rather than unwrapped so the impossible case stays
            // harmless instead of becoming a panic in a read surface.
            None => Self::UsageError(token.unwrap_or_default().to_owned()),
        }
    }

    /// The exit code **argv alone** decides, where argv decides one.
    ///
    /// `None` is the honest answer for a request whose outcome depends on
    /// something the command line does not contain: a `list` needs a session
    /// source, a launch candidate needs a launcher. Returning a number for
    /// those would publish an implementation's current behavior as though it
    /// were the contract, which is exactly the mistake this type refuses to
    /// make available.
    ///
    /// SC-022 fixes the one error code: `2` for a usage error, kept distinct
    /// from `1` so a caller can tell "you asked wrong" from "it went wrong".
    /// [`Request::MissingOperand`] is the same class and takes the same code.
    ///
    /// The helper surfaces answer `None` for the same reason `list` does:
    /// `_requests` may still refuse for want of an identity (a pinned `1`), and
    /// `_events-tail` has no completion at all — argv decides neither.
    #[must_use]
    pub fn exit_code(&self) -> Option<u8> {
        match self {
            Self::Version | Self::Help => Some(0),
            Self::UsageError(_) | Self::MissingOperand(_) => Some(2),
            Self::List(_)
            | Self::LaunchCandidate(_)
            | Self::Requests { .. }
            | Self::State { .. }
            | Self::EventsTail { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EVENTS_TAIL, REQUESTS, Request, STATE};
    use crate::filters::{ListArgs, Scope};
    use crate::requests::Mode;

    /// Every flag the rows name, as one list — used to prove DELEGATION rather
    /// than to re-test the grammar, which is [`crate::filters`]'s job.
    const EVERY_DOCUMENTED_FLAG: [&str; 10] = [
        "--running",
        "--all",
        "--stopped",
        "--needs-attn",
        "--needs-me",
        "--needs",
        "--attn",
        "--active",
        "--busy",
        "--json",
    ];

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn every_version_spelling_parses() {
        for arg in ["-V", "--version", "version"] {
            assert_eq!(Request::parse(&[arg.to_owned()]), Request::Version, "{arg}");
        }
    }

    #[test]
    fn every_help_spelling_parses() {
        for arg in ["-h", "--help", "help"] {
            assert_eq!(Request::parse(&[arg.to_owned()]), Request::Help, "{arg}");
        }
    }

    #[test]
    fn bare_argv_is_help() {
        assert_eq!(Request::parse(&[]), Request::Help);
    }

    #[test]
    fn sc_022_an_unknown_option_is_carried_verbatim() {
        // One token, alone: what argv does with tokens AFTER a recognised one
        // is explicitly unruled by SC-022, so nothing here may depend on it.
        assert_eq!(
            Request::parse(&argv(&["--frobnicate"])),
            Request::UsageError("--frobnicate".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&["-x"])),
            Request::UsageError("-x".to_owned())
        );
    }

    #[test]
    fn sc_022_a_top_level_bare_word_is_a_session_name_not_an_error() {
        // The colead veto, as a test: there is no unknown-subcommand phrase for
        // such a token to fall into. It is a launch candidate under the start
        // grammar, and launching is out of this slice — but MISCLASSIFYING it
        // would put a phrase into the contract that the row forbids.
        for word in ["my-feature", "frobnicate", "list-ish", "9lives"] {
            assert_eq!(
                Request::parse(&argv(&[word])),
                Request::LaunchCandidate(word.to_owned()),
                "{word}"
            );
        }
    }

    #[test]
    fn sc_022_option_shape_is_what_separates_the_two() {
        // The discriminator is the leading `-`, and nothing else.
        assert!(matches!(
            Request::parse(&argv(&["-"])),
            Request::UsageError(_)
        ));
        assert!(matches!(
            Request::parse(&argv(&["-frobnicate"])),
            Request::UsageError(_)
        ));
        assert!(matches!(
            Request::parse(&argv(&["frobnicate"])),
            Request::LaunchCandidate(_)
        ));
    }

    #[test]
    fn sc_022_argv_decides_the_usage_code_and_declines_to_decide_the_others() {
        assert_eq!(Request::Version.exit_code(), Some(0));
        assert_eq!(Request::Help.exit_code(), Some(0));
        assert_eq!(Request::UsageError("x".to_owned()).exit_code(), Some(2));
        // Neither of these is decidable from the command line alone, and a
        // number here would publish today's scaffold as tomorrow's contract.
        assert_eq!(Request::List(ListArgs::default()).exit_code(), None);
        assert_eq!(Request::LaunchCandidate("s".to_owned()).exit_code(), None);
    }

    #[test]
    fn sc_017a_bare_list_is_the_default_listing() {
        assert_eq!(
            Request::parse(&argv(&["list"])),
            Request::List(ListArgs::default())
        );
    }

    #[test]
    fn ls_is_the_same_command_as_list_for_every_argv_tail() {
        // SC-021 makes `ls` an alias of `list` — one command, two spellings.
        // Tails, not just the bare word: an alias that dispatches the same but
        // parses its arguments differently is not an alias.
        let tails: [&[&str]; 5] = [
            &[],
            &["--all"],
            &["--stopped", "--json"],
            &["--needs-attn", "--busy"],
            &["--frobnicate"],
        ];
        for tail in tails {
            let mut as_list = vec!["list"];
            as_list.extend_from_slice(tail);
            let mut as_ls = vec!["ls"];
            as_ls.extend_from_slice(tail);
            assert_eq!(
                Request::parse(&argv(&as_list)),
                Request::parse(&argv(&as_ls)),
                "{tail:?}"
            );
        }
    }

    #[test]
    fn every_documented_flag_reaches_the_one_parser_that_owns_it() {
        // DELEGATION, not the grammar: what each flag MEANS is pinned in
        // `filters`. What is pinned here is that `list` hands argv to that
        // parser untouched — a second grammar in this module is exactly the
        // drift this asserts against.
        for flag in EVERY_DOCUMENTED_FLAG {
            let expected = ListArgs::parse(&[flag]).expect("a documented flag");
            assert_eq!(
                Request::parse(&argv(&["list", flag])),
                Request::List(expected),
                "{flag}"
            );
        }
    }

    #[test]
    fn sc_521_the_whole_tail_is_parsed_not_just_the_first_flag() {
        // Only the first argument decides the COMMAND; inside `list` every
        // flag counts, and SC-521b's last-distinct-selector rule needs all of
        // them to have been seen.
        let Request::List(args) = Request::parse(&argv(&["list", "--all", "--stopped", "--json"]))
        else {
            panic!("a list request");
        };
        assert_eq!(args.selection.scope, Scope::Stopped);
        assert!(args.json);
    }

    #[test]
    fn sc_022_an_unknown_flag_in_a_list_tail_is_a_usage_error() {
        let request = Request::parse(&argv(&["list", "--all", "--frobnicate"]));
        assert_eq!(request, Request::UsageError("--frobnicate".to_owned()));
        assert_eq!(request.exit_code(), Some(2));
    }

    #[test]
    fn sc_022_a_bare_word_in_a_list_tail_is_a_usage_error_unlike_at_top_level() {
        // The row draws the line by POSITION, not by shape: the same token is a
        // session name at top level and a usage error inside a list tail.
        let in_tail = Request::parse(&argv(&["list", "my-feature"]));
        assert_eq!(in_tail, Request::UsageError("my-feature".to_owned()));
        assert_eq!(in_tail.exit_code(), Some(2));
        assert_eq!(
            Request::parse(&argv(&["my-feature"])),
            Request::LaunchCandidate("my-feature".to_owned()),
            "the same word at top level is not an error at all"
        );
    }

    #[test]
    fn the_helper_spellings_all_begin_with_an_underscore() {
        // Not decoration: this is the property that keeps SC-022 whole.
        // `_validate_session_name` forbids a leading underscore, so no legal
        // session name can reach these arms — a `requests`/`events-tail`
        // spelling would have taken two real names out of the launch grammar.
        for spelling in [REQUESTS, EVENTS_TAIL, STATE] {
            assert!(spelling.starts_with('_'), "{spelling}");
            assert!(
                !spelling.starts_with('-'),
                "{spelling}: not option-shaped either, or SC-022's first arm would take it"
            );
        }
    }

    #[test]
    fn requests_takes_a_directory_and_an_optional_mode() {
        assert_eq!(
            Request::parse(&argv(&[REQUESTS, "/s/tg1"])),
            Request::Requests {
                dir: "/s/tg1".into(),
                mode: Mode::Mine
            },
            "the absent mode is `mine`, as the helper's ${{1:-mine}} is"
        );
        for (token, mode) in [
            ("mine", Mode::Mine),
            ("inbox", Mode::Inbox),
            ("all", Mode::All),
        ] {
            assert_eq!(
                Request::parse(&argv(&[REQUESTS, "/s/tg1", token])),
                Request::Requests {
                    dir: "/s/tg1".into(),
                    mode
                },
                "{token}"
            );
        }
    }

    #[test]
    fn events_tail_takes_a_directory_and_nothing_else() {
        assert_eq!(
            Request::parse(&argv(&[EVENTS_TAIL, "/s/tg1"])),
            Request::EventsTail {
                dir: "/s/tg1".into()
            }
        );
    }

    #[test]
    fn a_helper_surface_with_no_directory_is_a_missing_operand_at_two() {
        for spelling in [REQUESTS, EVENTS_TAIL, STATE] {
            let request = Request::parse(&argv(&[spelling]));
            assert_eq!(request, Request::MissingOperand(spelling));
            assert_eq!(request.exit_code(), Some(2), "{spelling}");
        }
    }

    #[test]
    fn state_takes_a_directory_and_keeps_the_rest_for_the_state_module() {
        // argv is not validated here: the usage text is the helper's own and
        // lives beside the rule in `crate::state`. The parser only splits.
        assert_eq!(
            Request::parse(&argv(&[STATE, "/s/tg1", "blocked", "on", "x"])),
            Request::State {
                dir: "/s/tg1".into(),
                tail: argv(&["blocked", "on", "x"]),
            }
        );
        assert_eq!(
            Request::parse(&argv(&[STATE, "/s/tg1"])).exit_code(),
            None,
            "argv alone does not decide a declaration's outcome"
        );
    }

    #[test]
    fn a_bad_mode_token_is_the_usage_code_and_not_the_frozen_one() {
        // D2's split: the pinned `1` belongs to the identity refusal, which argv
        // cannot see. An unrecognised mode is a usage error like any other.
        let request = Request::parse(&argv(&[REQUESTS, "/s/tg1", "bogus"]));
        assert_eq!(request, Request::UsageError("bogus".to_owned()));
        assert_eq!(request.exit_code(), Some(2));
        assert_ne!(request.exit_code(), Some(1));
    }

    #[test]
    fn a_token_past_the_helper_grammar_is_a_usage_error_carrying_that_token() {
        assert_eq!(
            Request::parse(&argv(&[REQUESTS, "/s/tg1", "all", "extra"])),
            Request::UsageError("extra".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[EVENTS_TAIL, "/s/tg1", "extra"])),
            Request::UsageError("extra".to_owned())
        );
    }

    #[test]
    fn a_directory_is_taken_verbatim_however_it_is_spelled() {
        // The runner hands over whatever `<AE_HOME>/sessions/<name>` expanded
        // to. Nothing here normalises it: rewriting an operator's path is a
        // decision no row makes, and a mode-shaped directory name is still a
        // directory because position decides, not shape.
        for path in ["/s/tg1", "relative/tg1", "all", "--json", ""] {
            assert_eq!(
                Request::parse(&argv(&[REQUESTS, path, "all"])),
                Request::Requests {
                    dir: path.into(),
                    mode: Mode::All
                },
                "{path}"
            );
        }
    }

    #[test]
    fn a_helper_surface_declines_to_decide_its_exit_code_from_argv() {
        for words in [vec![REQUESTS, "/s/tg1", "all"], vec![EVENTS_TAIL, "/s/tg1"]] {
            assert_eq!(
                Request::parse(&argv(&words)).exit_code(),
                None,
                "{words:?}: an identity refusal and a follow are not argv facts"
            );
        }
    }

    #[test]
    fn a_list_that_parsed_is_never_a_usage_error() {
        for tail in [vec!["list"], vec!["ls", "--all", "--json"]] {
            assert_ne!(
                Request::parse(&argv(&tail)).exit_code(),
                Some(2),
                "{tail:?}"
            );
        }
    }
}
