//! The tmux version floor, and the refusal below it.
//!
//! ae draws its own surfaces with tmux — `display-menu`, the popup positions it
//! accepts, the session-scoped status formats and the per-window border styles
//! — so the version of the tmux that will run them decides whether they can be
//! drawn at all. The check is a read and a comparison; it never restarts a
//! server and never upgrades one, because both are the operator's call on a
//! machine that may be running other people's sessions.
//!
//! Two tmuxes can answer, and the gate asks whichever one this invocation will
//! actually use: a RUNNING server keeps executing the binary that started it
//! (`display-message -p '#{version}'`), while a launch that will create a new
//! server gets whatever `tmux -V` is on `PATH`. The two disagree exactly when
//! an upgrade has happened and the answer matters most.

use crate::inventory::ServerId;
use crate::meta::Selector;

/// The oldest tmux ae runs on.
///
/// 3.4, measured against the 3.4 manual: every option and format this design
/// uses exists there. It is also what the current Ubuntu LTS ships, so the
/// common Linux install needs nothing beyond `apt install tmux`.
pub const REQUIRED: Version = Version { major: 3, minor: 4 };

/// The exit a refused gate takes — "everything else", not a usage error.
pub const EXIT_REFUSED: u8 = 1;

/// The `major.minor` of a tmux release, which is all the floor compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// `3` of `3.4`.
    pub major: u32,
    /// `4` of `3.4`.
    pub minor: u32,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// What tmux answered `#{version}` with, once read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// A release ae can compare — `3.4`, `3.5a`, `next-3.8`.
    Release(Version),
    /// A development build that is ahead of every release by construction.
    Development,
    /// A spelling this module will not guess at.
    Unreadable,
}

/// WHICH tmux answered the gate, and what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// A running server answered `#{version}`. Its binary is the one that will
    /// draw, whatever `PATH` now holds.
    Server(String),
    /// No server is running there, so the `tmux` on `PATH` — the binary that
    /// would start one — answered `-V` instead.
    Executable(String),
    /// Neither answered: no server, and no runnable `tmux`.
    Silent,
    /// A server was addressed and could not be asked — a permission error, a
    /// refused connection, an unreadable answer. Its version is UNKNOWN, and
    /// the `PATH` binary does not stand in for it: that binary is not the one
    /// this server is running.
    Unreachable,
}

impl Probe {
    /// The version text this probe carries, or empty when nothing answered.
    #[must_use]
    pub fn found(&self) -> &str {
        match self {
            Self::Server(found) | Self::Executable(found) => found.trim(),
            Self::Silent | Self::Unreachable => "",
        }
    }

    /// WHICH tmux this reading is about, so a report never leaves the reader to
    /// assume it is theirs.
    ///
    /// A machine can hold several: a private `-L` server on an old binary and a
    /// newer one on `PATH` disagree, and a line that named neither would look
    /// like a contradiction of the refusal a launch prints.
    #[must_use]
    pub const fn source(&self) -> &'static str {
        match self {
            Self::Server(_) | Self::Unreachable => "(the server ae would use)",
            Self::Executable(_) => "(on PATH)",
            Self::Silent => "",
        }
    }

    /// Whether the tmux this probe read clears [`REQUIRED`].
    #[must_use]
    pub fn clears_floor(&self) -> bool {
        match self {
            Self::Server(found) | Self::Executable(found) => clears(found),
            // Nothing answered, so nothing was proven: the gate fails closed.
            Self::Silent | Self::Unreachable => false,
        }
    }
}

/// The development spelling tmux reports from its own git tip.
const DEVELOPMENT: &str = "master";

/// The prefix tmux's pre-release builds carry before their release number.
const NEXT_PREFIX: &str = "next-";

/// Read `#{version}` as the floor compares it.
///
/// The trailing letter of a point release (`3.4a`) is dropped: it never moves
/// the pair the floor is written in, and a floor that pretended otherwise
/// would refuse `3.4` for being older than `3.4a`.
///
/// ```
/// use ae::tmux_floor::{Reading, Version, read};
/// assert_eq!(read("3.4"), Reading::Release(Version { major: 3, minor: 4 }));
/// assert_eq!(read("3.7b"), Reading::Release(Version { major: 3, minor: 7 }));
/// assert_eq!(read("next-3.8"), Reading::Release(Version { major: 3, minor: 8 }));
/// assert_eq!(read("master"), Reading::Development);
/// assert_eq!(read("openbsd"), Reading::Unreadable);
/// ```
#[must_use]
pub fn read(raw: &str) -> Reading {
    let text = raw.trim();
    if text == DEVELOPMENT {
        return Reading::Development;
    }
    let text = text.strip_prefix(NEXT_PREFIX).unwrap_or(text);
    let Some((major, rest)) = text.split_once('.') else {
        return Reading::Unreadable;
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    // The remainder is ONE point-release letter or nothing: `3.4a` is a release,
    // `3.4-rc` and `3.4evil` are spellings this module has never seen.
    let tail = &rest[digits.len()..];
    let suffix_is_a_point_release =
        tail.is_empty() || (tail.len() == 1 && tail.starts_with(|c: char| c.is_ascii_alphabetic()));
    if digits.is_empty() || !suffix_is_a_point_release {
        return Reading::Unreadable;
    }
    match (major.parse(), digits.parse()) {
        (Ok(major), Ok(minor)) => Reading::Release(Version { major, minor }),
        _ => Reading::Unreadable,
    }
}

/// Whether `raw` clears [`REQUIRED`].
///
/// ```
/// assert!(ae::tmux_floor::clears("3.4"));
/// assert!(ae::tmux_floor::clears("4.0"));
/// assert!(!ae::tmux_floor::clears("3.3a"));
/// assert!(!ae::tmux_floor::clears("2.9"));
/// ```
#[must_use]
pub fn clears(raw: &str) -> bool {
    match read(raw) {
        Reading::Release(found) => found >= REQUIRED,
        Reading::Development => true,
        Reading::Unreadable => false,
    }
}

/// How a server is named back to the operator who has to fix it.
///
/// ```
/// use ae::inventory::ServerId;
/// assert_eq!(ae::tmux_floor::server_label(&ServerId::Ambient), "the current server ($TMUX)");
/// ```
#[must_use]
pub fn server_label(server: &ServerId) -> String {
    match server {
        ServerId::Ambient => "the current server ($TMUX)".to_owned(),
        ServerId::Selected(Selector::Name(name)) => format!("-L {name}"),
        ServerId::Selected(Selector::Socket(path)) => format!("-S {}", path.display()),
    }
}

/// What a server that did not answer `#{version}` is reported as.
pub const UNANSWERED: &str = "(no answer)";

/// How to get a tmux that clears the floor, on each platform ae runs on.
///
/// Named rather than described: the packaged route reaches the floor on both,
/// and an operator reading a bare "upgrade tmux" has to go and find that out.
const HOW_TO_GET_IT: &str = concat!(
    "  macOS:  brew install tmux\n",
    "  Linux:  apt install tmux   (Ubuntu 24.04 ships 3.4, which clears the floor;\n",
    "          an older distro needs its backport, a newer package, or a source build).",
);

/// The same two routes as [`HOW_TO_GET_IT`], on ONE line, for the report rows
/// that live in a column layout and cannot carry a block.
const ROUTE_HINT: &str = "(macOS: brew install tmux; Linux: apt install tmux)";

/// What the floor makes of a probe, for the surfaces that REPORT rather than
/// refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The tmux that answered clears [`REQUIRED`].
    Ok,
    /// It answered, and it is older than the floor — or it answered a spelling
    /// this module will not guess at, which is the same fail-closed outcome.
    Below,
    /// Nothing answered: no server, and no runnable `tmux`.
    Missing,
}

impl Verdict {
    /// The word a report column prints.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Below => "below",
            Self::Missing => "missing",
        }
    }
}

impl Probe {
    /// What the floor makes of this probe.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        match self {
            Self::Silent | Self::Unreachable => Verdict::Missing,
            _ if self.clears_floor() => Verdict::Ok,
            _ => Verdict::Below,
        }
    }
}

/// The consequence a below-floor tmux carries, said once so all three
/// reporting surfaces say it the same way.
const CONSEQUENCE: &str = "ae will refuse to launch a session until this is fixed";

/// One line naming the tmux ae found and what the floor makes of it — the
/// second line of `ae version`, and the detail of `ae doctor`'s floor row.
///
/// ```
/// use ae::tmux_floor::{Probe, summary};
/// assert_eq!(
///     summary(&Probe::Executable("3.7b".to_owned())),
///     "tmux 3.7b (on PATH) — clears the 3.4 floor"
/// );
/// assert!(summary(&Probe::Silent).starts_with("tmux not found"));
/// ```
#[must_use]
pub fn summary(probe: &Probe) -> String {
    let source = probe.source();
    match probe.verdict() {
        Verdict::Ok => format!(
            "tmux {} {source} — clears the {REQUIRED} floor",
            probe.found()
        ),
        Verdict::Below => format!(
            "tmux {} {source} — BELOW the {REQUIRED} floor; {CONSEQUENCE}",
            probe.found()
        ),
        Verdict::Missing if matches!(probe, Probe::Unreachable) => {
            format!("tmux {source} did not answer — ae needs {REQUIRED} or newer; {CONSEQUENCE}")
        }
        Verdict::Missing => format!("tmux not found — ae needs {REQUIRED} or newer; {CONSEQUENCE}"),
    }
}

/// [`summary`] plus the one-line route, for a report row in a column layout.
///
/// ```
/// use ae::tmux_floor::{Probe, row_detail};
/// assert_eq!(
///     row_detail(&Probe::Executable("3.7b".to_owned())),
///     "tmux 3.7b (on PATH) — clears the 3.4 floor"
/// );
/// assert!(row_detail(&Probe::Executable("3.3a".to_owned())).contains("brew install tmux"));
/// ```
#[must_use]
pub fn row_detail(probe: &Probe) -> String {
    match probe.verdict() {
        Verdict::Ok => summary(probe),
        _ => format!("{} {ROUTE_HINT}", summary(probe)),
    }
}

/// The warning a PUBLISH prints when the machine's tmux will not run what was
/// just installed, or `None` when it will.
///
/// A warning and not a refusal: the install has to succeed on a machine below
/// the floor, because `ae version` is how the operator sees the problem and
/// `ae upgrade` is how they leave it behind.
#[must_use]
pub fn advisory(probe: &Probe) -> Option<String> {
    (probe.verdict() != Verdict::Ok)
        .then(|| format!("warning: {}\n{HOW_TO_GET_IT}\n", summary(probe)))
}

/// The refusal `command` prints for `probe` on `server`, terminating newline
/// included.
///
/// FOUR failures, so four headlines: an old SERVER is fixed by starting a new
/// one on an upgraded binary, an old EXECUTABLE by installing a newer tmux,
/// silence by having a tmux at all, and a server that could not be asked by
/// fixing the connection to it.
///
/// ```
/// use ae::inventory::ServerId;
/// use ae::tmux_floor::{Probe, refusal};
/// let text = refusal("orchestrator", &Probe::Server("3.3a".to_owned()), &ServerId::Ambient);
/// assert!(text.contains("found:    3.3a"));
/// assert!(text.contains("required: 3.4 or newer"));
/// ```
#[must_use]
pub fn refusal(command: &str, probe: &Probe, server: &ServerId) -> String {
    let (headline, guidance) = match probe {
        Probe::Server(_) => (
            "the tmux server it would use is older than ae runs on",
            "Install a newer tmux, then start a NEW server with it — ae never restarts a\nrunning server for you.",
        ),
        Probe::Executable(_) => (
            "the tmux on PATH is older than ae runs on",
            "Install a newer tmux — this launch would have started a server with this one.",
        ),
        Probe::Silent => (
            "no tmux answered, so ae cannot tell which version would run",
            "Install tmux, or start the server this command addresses — ae never starts or\nrestarts a server for you.",
        ),
        Probe::Unreachable => (
            "the tmux server it would use could not be asked its version",
            "Fix the connection to that server, then retry — ae will not stand in the PATH\ntmux's version for a server that is running some other binary.",
        ),
    };
    let found = match probe.found() {
        "" => UNANSWERED,
        text => text,
    };
    format!(
        "ae {command}: {headline}.\n  \
         found:    {found}\n  \
         required: {REQUIRED} or newer\n  \
         server:   {}\n\
         {guidance}\n\
         {HOW_TO_GET_IT}\n",
        server_label(server)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        EXIT_REFUSED, HOW_TO_GET_IT, Probe, REQUIRED, ROUTE_HINT, Reading, UNANSWERED, Verdict,
        Version, advisory, clears, read, refusal, row_detail, server_label, summary,
    };
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    use std::path::PathBuf;

    #[test]
    fn the_floor_is_the_ruled_three_four() {
        assert_eq!(REQUIRED, Version { major: 3, minor: 4 });
        assert_eq!(REQUIRED.to_string(), "3.4");
    }

    /// The whole table the gate is written against, one row per spelling a real
    /// tmux reports: the LTS distro release, the point release ae runs, the
    /// pre-release prefix, and the git tip.
    #[test]
    fn the_parse_table_reads_every_spelling_tmux_reports() {
        let release = |major, minor| Reading::Release(Version { major, minor });
        for (spelling, reading, clears_floor) in [
            ("2.9", release(2, 9), false),
            ("3.3a", release(3, 3), false),
            ("3.4", release(3, 4), true),
            ("3.4a", release(3, 4), true),
            ("3.5a", release(3, 5), true),
            ("3.6", release(3, 6), true),
            ("3.7", release(3, 7), true),
            ("3.7b", release(3, 7), true),
            ("next-3.8", release(3, 8), true),
            ("3.8", release(3, 8), true),
            ("4.0", release(4, 0), true),
            ("10.0", release(10, 0), true),
            ("master", Reading::Development, true),
        ] {
            assert_eq!(read(spelling), reading, "{spelling}");
            assert_eq!(clears(spelling), clears_floor, "{spelling}");
        }
    }

    #[test]
    fn a_point_release_letter_never_moves_the_pair_the_floor_compares() {
        // 3.4a is 3.4 with fixes, so a floor of 3.4 must admit it.
        for spelling in ["3.4", "3.4a", "3.4b"] {
            assert!(clears(spelling), "{spelling}");
        }
        // And 3.3a is still below it.
        for spelling in ["3.3", "3.3a", "2.9", "1.8"] {
            assert!(!clears(spelling), "{spelling}");
        }
    }

    #[test]
    fn surrounding_whitespace_is_the_capture_and_not_the_version() {
        assert!(clears(" 3.7b\n"));
        assert_eq!(read("\t3.4 "), Reading::Release(REQUIRED));
    }

    #[test]
    fn a_spelling_this_module_cannot_read_fails_closed() {
        for spelling in [
            "", "   ", "openbsd", "3", "3.", "3.x", "3.7-rc1", "x.y", "3.7evil", "3.7ab",
        ] {
            assert_eq!(read(spelling), Reading::Unreadable, "{spelling}");
            assert!(!clears(spelling), "{spelling}");
        }
    }

    #[test]
    fn a_probe_carries_the_version_of_whichever_tmux_answered() {
        assert_eq!(Probe::Server(" 3.7b\n".to_owned()).found(), "3.7b");
        assert_eq!(Probe::Executable("3.3a".to_owned()).found(), "3.3a");
        assert_eq!(Probe::Silent.found(), "");

        assert!(Probe::Server("3.7".to_owned()).clears_floor());
        assert!(Probe::Executable("3.7b".to_owned()).clears_floor());
        assert!(!Probe::Server("3.3a".to_owned()).clears_floor());
        assert!(!Probe::Executable("2.9".to_owned()).clears_floor());
        assert!(
            !Probe::Silent.clears_floor(),
            "nothing answered, so nothing was proven"
        );
    }

    #[test]
    fn the_refusal_carries_found_required_and_the_server_it_asked() {
        let text = refusal(
            "orchestrator",
            &Probe::Server("3.3a".to_owned()),
            &ServerId::Ambient,
        );
        assert!(
            text.starts_with("ae orchestrator: the tmux server"),
            "{text}"
        );
        assert!(text.contains("found:    3.3a"), "{text}");
        assert!(text.contains("required: 3.4 or newer"), "{text}");
        assert!(
            text.contains("server:   the current server ($TMUX)"),
            "{text}"
        );
        assert!(
            text.contains("ae never restarts a\nrunning server for you"),
            "the gate says what it will NOT do: {text}"
        );
        assert!(text.ends_with('\n'));
    }

    /// The three answers are three different fixes, so they are three different
    /// headlines — an operator who reads "upgrade the server" when there is no
    /// server goes looking for one that never existed.
    #[test]
    fn each_probe_names_the_fix_that_belongs_to_it() {
        let server = refusal("x", &Probe::Server("3.3a".to_owned()), &ServerId::Ambient);
        let executable = refusal(
            "x",
            &Probe::Executable("3.3a".to_owned()),
            &ServerId::Ambient,
        );
        let silent = refusal("x", &Probe::Silent, &ServerId::Ambient);

        assert!(server.contains("start a NEW server"), "{server}");
        assert!(
            !executable.contains("restarts a running server"),
            "{executable}"
        );
        assert!(
            executable.contains("this launch would have started a server with this one"),
            "{executable}"
        );
        assert!(silent.contains("no tmux answered"), "{silent}");
        assert!(
            silent.contains(&format!("found:    {UNANSWERED}")),
            "{silent}"
        );
        assert!(
            !server.contains("no tmux answered"),
            "an answered version is an upgrade problem: {server}"
        );
    }

    /// The refusal names the packaged route on each platform rather than
    /// saying "upgrade tmux": an operator reading the bare instruction has to
    /// go and find out which command reaches the floor on their machine.
    #[test]
    fn the_guidance_names_a_route_to_the_floor_on_both_platforms() {
        let text = refusal(
            "orchestrator",
            &Probe::Server("3.3a".to_owned()),
            &ServerId::Ambient,
        );
        assert!(text.contains(HOW_TO_GET_IT), "{text}");
        assert!(text.contains("brew install tmux"), "{text}");
        assert!(text.contains("apt install tmux"), "{text}");
        assert!(text.contains("Ubuntu 24.04 ships 3.4"), "{text}");
        // INDENTED, both of them: the block is a two-item list under the
        // refusal, and a `\`-continued literal silently eats the first line's
        // indent, which is exactly how it read wrong once.
        for line in ["  macOS:", "  Linux:"] {
            assert!(
                text.lines().any(|drawn| drawn.starts_with(line)),
                "{line} is not indented in:\n{text}"
            );
        }
    }

    #[test]
    fn every_addressed_server_names_the_flags_that_would_reach_it() {
        assert_eq!(
            server_label(&ServerId::Selected(Selector::Name("ae-dev".to_owned()))),
            "-L ae-dev"
        );
        assert_eq!(
            server_label(&ServerId::Selected(Selector::Socket(PathBuf::from(
                "/tmp/s"
            )))),
            "-S /tmp/s"
        );
        assert_eq!(
            server_label(&ServerId::Ambient),
            "the current server ($TMUX)"
        );
    }

    /// The three reporting surfaces state ONE verdict, so the verdict is a
    /// function of the probe and nothing else.
    #[test]
    fn every_probe_has_exactly_one_verdict() {
        for (probe, verdict) in [
            (Probe::Server("3.7b".to_owned()), Verdict::Ok),
            (Probe::Executable("3.8".to_owned()), Verdict::Ok),
            (Probe::Server("master".to_owned()), Verdict::Ok),
            (Probe::Server("3.3a".to_owned()), Verdict::Below),
            (Probe::Executable("2.9".to_owned()), Verdict::Below),
            // An unreadable spelling proves nothing, so it reads as below.
            (Probe::Executable("openbsd".to_owned()), Verdict::Below),
            (Probe::Silent, Verdict::Missing),
        ] {
            assert_eq!(probe.verdict(), verdict, "{probe:?}");
        }
        assert_eq!(Verdict::Ok.as_str(), "ok");
        assert_eq!(Verdict::Below.as_str(), "below");
        assert_eq!(Verdict::Missing.as_str(), "missing");
    }

    /// `ae version`'s second line: what was found, and what the floor says.
    #[test]
    fn the_summary_names_the_version_the_floor_and_the_consequence() {
        assert_eq!(
            summary(&Probe::Server("3.7b".to_owned())),
            "tmux 3.7b (the server ae would use) — clears the 3.4 floor"
        );
        assert_eq!(
            summary(&Probe::Executable("3.7b".to_owned())),
            "tmux 3.7b (on PATH) — clears the 3.4 floor"
        );
        // A server that could not be asked is not a machine without tmux.
        let unreachable = summary(&Probe::Unreachable);
        assert!(unreachable.contains("did not answer"), "{unreachable}");
        assert!(
            unreachable.contains("the server ae would use"),
            "the reading names WHICH tmux it is about: {unreachable}"
        );
        let below = summary(&Probe::Executable("3.3a".to_owned()));
        assert!(below.contains("tmux 3.3a"), "{below}");
        assert!(below.contains("BELOW the 3.4 floor"), "{below}");
        assert!(below.contains("refuse to launch a session"), "{below}");
        let missing = summary(&Probe::Silent);
        assert!(missing.starts_with("tmux not found"), "{missing}");
        assert!(missing.contains("3.4 or newer"), "{missing}");
    }

    /// `ae doctor`'s row lives in a column layout, so its detail is ONE line —
    /// and it still names both routes.
    #[test]
    fn the_doctor_detail_is_one_line_and_still_names_both_routes() {
        let detail = row_detail(&Probe::Executable("3.3a".to_owned()));
        assert!(!detail.contains('\n'), "{detail}");
        assert!(detail.contains("brew install tmux"), "{detail}");
        assert!(detail.contains("apt install tmux"), "{detail}");
        // A machine that clears the floor is told nothing to do.
        let ok = row_detail(&Probe::Server("3.7b".to_owned()));
        assert_eq!(ok, summary(&Probe::Server("3.7b".to_owned())));
        assert!(!ok.contains("brew"), "{ok}");
    }

    /// The publish warning: present below the floor, ABSENT above it, and never
    /// a refusal — the install has to land either way.
    #[test]
    fn the_publish_advisory_appears_only_below_the_floor() {
        assert_eq!(advisory(&Probe::Server("3.7b".to_owned())), None);
        let text = advisory(&Probe::Executable("3.3a".to_owned())).unwrap_or_default();
        assert!(text.starts_with("warning: tmux 3.3a"), "{text}");
        assert!(text.contains(HOW_TO_GET_IT), "{text}");
        assert!(text.ends_with('\n'), "{text}");
        assert!(
            !text.contains("required:"),
            "the advisory reports, it does not refuse: {text}"
        );
        assert!(advisory(&Probe::Silent).is_some());
    }

    /// The block and the one-liner are two renderings of the SAME two routes.
    #[test]
    fn both_route_spellings_name_the_same_two_routes() {
        for text in [HOW_TO_GET_IT, ROUTE_HINT] {
            assert!(text.contains("brew install tmux"), "{text}");
            assert!(text.contains("apt install tmux"), "{text}");
        }
    }

    #[test]
    fn a_refused_gate_is_a_failure_and_not_a_usage_error() {
        assert_eq!(EXIT_REFUSED, 1);
    }
}
