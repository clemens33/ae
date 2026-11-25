//! The release a tool start measured of its own harness.
//!
//! `_run` asks the seat's harness for `--version` once per tool start and
//! records the answer as the seat's `version.<slot>` row: a release word, or
//! `unknown <why>` when the answer is missing, late, garbled or never asked for.
//! This module is the PURE half: the one parser of an answer, the one grammar of
//! the row, and the words `ae list` and `ae doctor` draw it with. The question
//! is `transport::run_tool_version`; the row's reader and writer are in `meta`.
//!
//! An answer is hostile input — a child's stdout — and a row is hostile
//! persisted state (a hand edit, a crashed write): both are parsed here and
//! both are fuzzed (`tool_version`, and `meta_parse` for the row).

use crate::tool::ToolKind;

/// The most bytes of an answer ae reads; a longer answer is [`Why::Oversize`].
/// A real `--version` is one short line (measured: 17 to 33 bytes).
pub const ANSWER_CAP: usize = crate::transport::VERSION_ANSWER_CAP;

/// Release parts of a core `N.N` to `N.N.N.N`.
const PARTS: std::ops::RangeInclusive<usize> = 2..=4;
/// Digits of one part: at most nine, so a part always fits a `u32`.
const PART_DIGITS: usize = 9;
/// The longest `-pre` or `+build` identifier kept after the core.
const SUFFIX_MAX: usize = 32;

/// Why a start recorded no release. The closed vocabulary of the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// No such program on the child's `PATH`.
    Absent,
    /// The question outlived its deadline and the child was killed.
    Timeout,
    /// The program did not run, or exited unsuccessfully.
    Failed,
    /// The answer was longer than [`ANSWER_CAP`].
    Oversize,
    /// The answer holds no release word.
    Unparseable,
    /// The seat's program is not a harness ae classifies (a wrapper such as
    /// `cc` or `codex-mic`), so it was never asked and nothing is guessed.
    Unclassified,
}

impl Why {
    /// Every reason, for the readers and the tests that must cover them all.
    pub const ALL: [Self; 6] = [
        Self::Absent,
        Self::Timeout,
        Self::Failed,
        Self::Oversize,
        Self::Unparseable,
        Self::Unclassified,
    ];

    /// The word the row and the cells carry.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::Oversize => "oversize",
            Self::Unparseable => "unparseable",
            Self::Unclassified => "unclassified",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|why| why.word() == word)
    }
}

/// What one tool start learned of its harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// The release the harness printed, as [`is_release`] judges it.
    Release(String),
    /// Nothing was measured, and why.
    Unknown(Why),
}

/// The word the row carries where no release stands.
const UNKNOWN: &str = "unknown";

impl Seen {
    /// The value of the `version.<slot>` row: `2.1.330` or `unknown <why>`.
    #[must_use]
    pub fn row(&self) -> String {
        match self {
            Self::Release(release) => release.clone(),
            Self::Unknown(why) => format!("{UNKNOWN} {}", why.word()),
        }
    }

    /// The one reader of a row's value; `None` for anything [`Seen::row`]
    /// would not have written.
    #[must_use]
    pub fn parse_row(value: &str) -> Option<Self> {
        if let Some(why) = value
            .strip_prefix(UNKNOWN)
            .and_then(|rest| rest.strip_prefix(' '))
        {
            return Why::from_word(why).map(Self::Unknown);
        }
        is_release(value).then(|| Self::Release(value.to_owned()))
    }
}

/// Whether `word` is a release: a core of two to four dot-separated parts of
/// one to nine ASCII digits, then optionally `-` or `+` and an identifier of
/// at most [`SUFFIX_MAX`] bytes of ASCII letters, digits, `.`, `-` and `+`, so a
/// pre-release with build metadata (`1.2.3-rc.1+build.4`) is one release.
/// `2.1.330`, `0.172.0` and `1.4.25-R9361.1` are releases; `2`, `1.x`, `.1.2`
/// and `1.2.` are not.
#[must_use]
pub fn is_release(word: &str) -> bool {
    let (core, suffix) = match word.find(['-', '+']) {
        Some(at) => (&word[..at], Some(&word[at + 1..])),
        None => (word, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let core_ok = PARTS.contains(&parts.len())
        && parts.iter().all(|part| {
            (1..=PART_DIGITS).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit())
        });
    let suffix_ok = suffix.is_none_or(|id| {
        (1..=SUFFIX_MAX).contains(&id.len())
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    });
    core_ok && suffix_ok
}

/// The release a `--version` answer names: the first release word, an optional
/// leading `v` dropped, of the first non-empty line. Measured shapes:
/// `2.1.332 (Claude Code)`, `codex-cli 0.172.0`, `opencode v2.4.1`, `1.5.17`,
/// `grok 1.0.92 (b06756fd6256)`, `Muse Code 1.4.25 (1.4.25-R9361.1)`.
///
/// # Errors
///
/// [`Why::Oversize`] past [`ANSWER_CAP`] bytes; [`Why::Unparseable`] for bytes
/// that are not UTF-8, an empty answer, or a first line with no release word.
pub fn parse_answer(bytes: &[u8]) -> Result<String, Why> {
    if bytes.len() > ANSWER_CAP {
        return Err(Why::Oversize);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Why::Unparseable)?;
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or(Why::Unparseable)?;
    line.split_whitespace()
        .map(|word| word.strip_prefix('v').unwrap_or(word))
        .find(|word| is_release(word))
        .map(str::to_owned)
        .ok_or(Why::Unparseable)
}

/// The cell `ae list` and `ae doctor` draw for one seat: what its start
/// measured, beside what the frozen fixtures prove for that tool, as
/// `claude 2.1.330 (measured 2.1.330)`, `claude unknown (timeout) (measured
/// 2.1.330)` or, for a tool no fixture names, `agy 1.5.17 (measured: none)`.
/// `label` is the seat's recorded binary, already safe to print.
#[must_use]
pub fn cell(label: &str, tool: ToolKind, seen: &Seen) -> String {
    let measured = tool.measured_version().map_or_else(
        || "(measured: none)".to_owned(),
        |m| format!("(measured {m})"),
    );
    match seen {
        Seen::Release(release) => format!("{label} {release} {measured}"),
        Seen::Unknown(why) => format!("{label} {UNKNOWN} ({}) {measured}", why.word()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_measured_answer_yields_its_release() {
        for (answer, release) in [
            ("2.1.332 (Claude Code)\n", "2.1.332"),
            ("codex-cli 0.172.0\n", "0.172.0"),
            ("opencode v2.4.1\n", "2.4.1"),
            ("1.5.17\n", "1.5.17"),
            ("grok 1.0.92 (b06756fd6256)\n", "1.0.92"),
            ("Muse Code 1.4.25 (1.4.25-R9361.1)\n", "1.4.25"),
            ("\n\n  0.35.0  \n", "0.35.0"),
            ("tool 0.172.0-alpha.3\n", "0.172.0-alpha.3"),
        ] {
            assert_eq!(
                parse_answer(answer.as_bytes()),
                Ok(release.to_owned()),
                "{answer:?}"
            );
        }
    }

    #[test]
    fn an_answer_without_a_release_is_unparseable_and_never_a_guess() {
        for answer in [
            "",
            "\n \t\n",
            "this is not a release\n",
            "node v22\n",
            "1\n",
            "1.x.3\n",
            ".1.2\n",
            "1.2.\n",
            "1.2.3.4.5\n",
            "1234567890.1\n",
            "1.2-\n",
            "1.2-bad!id\n",
            "Usage: tool [options]\n2.1.330\n",
        ] {
            assert_eq!(
                parse_answer(answer.as_bytes()),
                Err(Why::Unparseable),
                "{answer:?}"
            );
        }
        assert_eq!(
            parse_answer(&[0xff, 0xfe, b'1', b'.', b'2']),
            Err(Why::Unparseable)
        );
    }

    #[test]
    fn the_cap_is_inclusive_and_past_it_is_oversize() {
        let mut at_cap = b"2.1.330 ".to_vec();
        at_cap.resize(ANSWER_CAP, b'x');
        assert_eq!(parse_answer(&at_cap), Ok("2.1.330".to_owned()));
        at_cap.push(b'x');
        assert_eq!(parse_answer(&at_cap), Err(Why::Oversize));
    }

    #[test]
    fn a_release_has_two_to_four_parts_of_one_to_nine_digits() {
        for good in [
            "1.2",
            "1.2.3",
            "1.2.3.4",
            "999999999.0",
            "0.0.0",
            "1.2.3-rc.1",
            "1.2+b-7",
            "1.2.3-rc.1+build.4",
            "1.2.3+build.5-x",
        ] {
            assert!(is_release(good), "{good}");
        }
        for bad in [
            "",
            "1",
            "1..2",
            "1.2.3.4.5",
            "1.2.3-",
            "1.2.3+",
            "v1.2",
            "1.2 ",
            "٣.٤",
            "1.2.3-é",
        ] {
            assert!(!is_release(bad), "{bad:?}");
        }
        assert!(is_release(&format!("1.2-{}", "a".repeat(SUFFIX_MAX))));
        assert!(!is_release(&format!("1.2-{}", "a".repeat(SUFFIX_MAX + 1))));
        assert!(!is_release("1234567890.1"));
    }

    #[test]
    fn a_row_round_trips_and_nothing_else_reads() {
        let mut seen = vec![
            Seen::Release("2.1.330".to_owned()),
            Seen::Release("1.4.25-R9361.1".to_owned()),
        ];
        seen.extend(Why::ALL.map(Seen::Unknown));
        for seen in seen {
            assert_eq!(Seen::parse_row(&seen.row()), Some(seen.clone()), "{seen:?}");
        }
        for bad in [
            "",
            "unknown",
            "unknown ",
            "unknown nonsense",
            "unknown  timeout",
            "Unknown timeout",
            "unknown timeout ",
            "claude 2.1.330",
            "2.1.330 ",
            " 2.1.330",
            "v2.1.330",
            "unknown timeout\n",
        ] {
            assert_eq!(Seen::parse_row(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_cell_names_the_measured_release_or_none() {
        let seen = Seen::Release("2.1.332".to_owned());
        assert_eq!(
            cell("claude", ToolKind::Claude, &seen),
            format!(
                "claude 2.1.332 (measured {})",
                ToolKind::Claude
                    .measured_version()
                    .expect("claude is measured")
            )
        );
        assert_eq!(
            cell(
                "gemini",
                ToolKind::Gemini,
                &Seen::Release("0.35.0".to_owned())
            ),
            "gemini 0.35.0 (measured: none)"
        );
        assert_eq!(
            cell(
                "codex-mic",
                ToolKind::Unknown,
                &Seen::Unknown(Why::Unclassified)
            ),
            "codex-mic unknown (unclassified) (measured: none)"
        );
    }
}
