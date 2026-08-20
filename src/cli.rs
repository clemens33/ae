//! Argv parsing: the one place that turns a command line into an intent.
//!
//! Deliberately hand-rolled. A CLI argument parser is a dependency the skeleton
//! does not need, and #80's rule for the error crate — "no dependency exists
//! until a real error does" — applies here too. When the real command surface
//! arrives (P1+), revisit with a measured need.

use crate::filters::{ListArgs, UnknownFlag};

/// What an argv asks the binary to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Print the version line.
    Version,
    /// Print usage.
    Help,
    /// `list` (or its `ls` spelling) with the filters its flags selected.
    List(ListArgs),
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
    #[must_use]
    pub fn parse(args: &[String]) -> Self {
        match args.first().map(String::as_str) {
            None | Some("-h" | "--help" | "help") => Self::Help,
            Some("-V" | "--version" | "version") => Self::Version,
            Some("list" | "ls") => match ListArgs::parse(&args[1..]) {
                Ok(parsed) => Self::List(parsed),
                Err(UnknownFlag(token)) => Self::UsageError(token),
            },
            // SC-022, in the order the row states it: option-shaped first,
            // because everything left over is a name and not an error.
            Some(other) if other.starts_with('-') => Self::UsageError(other.to_owned()),
            Some(other) => Self::LaunchCandidate(other.to_owned()),
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
    #[must_use]
    pub fn exit_code(&self) -> Option<u8> {
        match self {
            Self::Version | Self::Help => Some(0),
            Self::UsageError(_) => Some(2),
            Self::List(_) | Self::LaunchCandidate(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Request;
    use crate::filters::{ListArgs, Scope};

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
