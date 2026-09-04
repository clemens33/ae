//! `_net-probe <host> [--port <n>]` — can THIS binary's resolver answer?

use std::io::Write;
use std::net::ToSocketAddrs as _;

use crate::Result;

/// The port a probe resolves for when none is given.
pub const DEFAULT_PORT: u16 = 443;

/// What a probe that could not resolve exits with.
pub const EXIT_UNRESOLVED: u8 = 1;

/// The result of one lookup, as a class rather than a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The resolver answered with this many addresses (never zero).
    Resolved(usize),
    /// The resolver returned an error.
    Unresolved,
    /// The resolver returned success and no addresses at all.
    NoAddresses,
}

impl Outcome {
    /// The one line this outcome prints.
    ///
    /// ```
    /// use ae::netprobe::Outcome;
    /// assert_eq!(Outcome::Resolved(2).line(), "ok 2");
    /// assert_eq!(Outcome::Unresolved.line(), "error: unresolved");
    /// assert_eq!(Outcome::NoAddresses.line(), "error: no-addresses");
    /// ```
    #[must_use]
    pub fn line(self) -> String {
        match self {
            Self::Resolved(count) => format!("ok {count}"),
            Self::Unresolved => "error: unresolved".to_owned(),
            Self::NoAddresses => "error: no-addresses".to_owned(),
        }
    }

    /// Whether this is the asked-for answer rather than a diagnostic.
    #[must_use]
    pub const fn resolved(self) -> bool {
        matches!(self, Self::Resolved(_))
    }

    /// The process exit code this outcome carries.
    ///
    /// ```
    /// use ae::netprobe::Outcome;
    /// assert_eq!(Outcome::Resolved(1).exit_code(), 0);
    /// assert_eq!(Outcome::Unresolved.exit_code(), 1);
    /// ```
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        if self.resolved() { 0 } else { EXIT_UNRESOLVED }
    }
}

/// Turn what the resolver did into the class that is reported.
///
/// Split from the lookup itself so the mapping is testable without a network:
/// every class here is reachable from a value, including the empty success no
/// resolver on this machine may ever produce.
///
/// Taken by REFERENCE because the error is never consumed: the class is the
/// whole report, and an `io::Error`'s message is the libc wording this
/// deliberately does not repeat.
///
/// ```
/// use ae::netprobe::{classify, Outcome};
/// assert_eq!(classify(&Ok(3)), Outcome::Resolved(3));
/// assert_eq!(classify(&Ok(0)), Outcome::NoAddresses);
/// assert_eq!(
///     classify(&Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
///     Outcome::Unresolved
/// );
/// ```
#[must_use]
pub fn classify(resolved: &std::io::Result<usize>) -> Outcome {
    match resolved {
        Ok(0) => Outcome::NoAddresses,
        Ok(count) => Outcome::Resolved(*count),
        Err(_) => Outcome::Unresolved,
    }
}

/// Resolve `host:port` and classify the answer.
#[must_use]
pub fn probe(host: &str, port: u16) -> Outcome {
    classify(&(host, port).to_socket_addrs().map(Iterator::count))
}

/// Run the probe and print its one line.
///
/// # Errors
///
/// Only if writing that line fails — a closed pipe or a gone terminal. A lookup
/// that failed is an OUTCOME, reported on stderr with a non-zero code, never an
/// `Err`: the instrument answering "no" is the instrument working.
pub fn run(host: &str, port: u16, out: &mut impl Write, err: &mut impl Write) -> Result<u8> {
    let outcome = probe(host, port);
    if outcome.resolved() {
        writeln!(out, "{}", outcome.line())?;
    } else {
        writeln!(err, "{}", outcome.line())?;
    }
    Ok(outcome.exit_code())
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PORT, EXIT_UNRESOLVED, Outcome, classify, probe, run};
    use std::io;

    #[test]
    fn the_three_classes_are_the_three_things_a_resolver_can_do() {
        assert_eq!(classify(&Ok(1)), Outcome::Resolved(1));
        assert_eq!(classify(&Ok(9)), Outcome::Resolved(9));
        // The empty success is its own class, and is the reason `classify`
        // takes a count rather than being fused into `probe`: no resolver here
        // produces it on demand, so a value is the only way to reach it.
        assert_eq!(classify(&Ok(0)), Outcome::NoAddresses);
        assert_eq!(
            classify(&Err(io::Error::from(io::ErrorKind::NotFound))),
            Outcome::Unresolved
        );
    }

    #[test]
    fn only_a_resolved_outcome_succeeds() {
        assert_eq!(Outcome::Resolved(1).exit_code(), 0);
        assert_eq!(Outcome::Unresolved.exit_code(), EXIT_UNRESOLVED);
        assert_eq!(Outcome::NoAddresses.exit_code(), EXIT_UNRESOLVED);
        assert!(Outcome::Resolved(1).resolved());
        assert!(!Outcome::Unresolved.resolved());
        assert!(!Outcome::NoAddresses.resolved());
    }

    /// The CALIBRATION half: a name that must resolve on any machine that can
    /// run this suite at all.
    #[test]
    fn a_name_every_machine_resolves_answers_ok() {
        let outcome = probe("localhost", DEFAULT_PORT);
        assert!(
            outcome.resolved(),
            "localhost did not resolve: {}",
            outcome.line()
        );
        assert!(outcome.line().starts_with("ok "));
    }

    /// The NEGATIVE half.
    #[test]
    fn a_reserved_name_that_cannot_exist_is_unresolved() {
        let outcome = probe("ae-net-probe-must-never-resolve.invalid", DEFAULT_PORT);
        assert_eq!(outcome, Outcome::Unresolved);
        assert_eq!(outcome.exit_code(), EXIT_UNRESOLVED);
    }

    /// Documents the instrument's LIMIT, executably: an IP literal answers
    /// without a resolver, so a green probe against one proves nothing about
    /// DNS or NSS.
    #[test]
    fn an_ip_literal_answers_without_asking_a_resolver() {
        assert_eq!(probe("127.0.0.1", DEFAULT_PORT), Outcome::Resolved(1));
    }

    #[test]
    fn the_answer_goes_to_stdout_and_a_failure_goes_to_stderr() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run("localhost", DEFAULT_PORT, &mut out, &mut err).unwrap();
        assert_eq!(code, 0);
        assert!(String::from_utf8(out).unwrap().starts_with("ok "));
        assert_eq!(String::from_utf8(err).unwrap(), "");

        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(
            "ae-net-probe-must-never-resolve.invalid",
            DEFAULT_PORT,
            &mut out,
            &mut err,
        )
        .unwrap();
        assert_eq!(code, EXIT_UNRESOLVED);
        assert_eq!(String::from_utf8(out).unwrap(), "");
        assert_eq!(String::from_utf8(err).unwrap(), "error: unresolved\n");
    }
}
