//! Argv parsing: the one place that turns a command line into an intent.
//!
//! Deliberately hand-rolled. A CLI argument parser is a dependency the skeleton
//! does not need, and #80's rule for the error crate — "no dependency exists
//! until a real error does" — applies here too. When the real command surface
//! arrives (P1+), revisit with a measured need.

/// What an argv asks the binary to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Print the version line.
    Version,
    /// Print usage.
    Help,
    /// The first argument is not one this binary knows.
    Unknown(String),
}

impl Request {
    /// Classify `args` — argv WITHOUT the program name.
    ///
    /// An empty argv is [`Request::Help`]: a multiplexer that prints nothing
    /// when invoked bare is a multiplexer nobody can discover.
    ///
    /// ```
    /// use ae::cli::Request;
    /// assert_eq!(Request::parse(&[]), Request::Help);
    /// assert_eq!(Request::parse(&["-V".to_owned()]), Request::Version);
    /// ```
    #[must_use]
    pub fn parse(args: &[String]) -> Self {
        match args.first().map(String::as_str) {
            None | Some("-h" | "--help" | "help") => Self::Help,
            Some("-V" | "--version" | "version") => Self::Version,
            Some(other) => Self::Unknown(other.to_owned()),
        }
    }

    /// The process exit code this request ends in.
    ///
    /// Unrecognised argv is `2` — the shell convention for a usage error, kept
    /// distinct from `1` so a caller can tell "you asked wrong" from "it went
    /// wrong".
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Version | Self::Help => 0,
            Self::Unknown(_) => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Request;

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
    fn an_unrecognised_argument_is_carried_verbatim() {
        assert_eq!(
            Request::parse(&["--frobnicate".to_owned(), "--version".to_owned()]),
            Request::Unknown("--frobnicate".to_owned())
        );
    }

    #[test]
    fn only_the_first_argument_decides() {
        assert_eq!(
            Request::parse(&["--version".to_owned(), "--help".to_owned()]),
            Request::Version
        );
    }

    #[test]
    fn exit_codes_separate_success_from_usage_error() {
        assert_eq!(Request::Version.exit_code(), 0);
        assert_eq!(Request::Help.exit_code(), 0);
        assert_eq!(Request::Unknown("x".to_owned()).exit_code(), 2);
    }
}
