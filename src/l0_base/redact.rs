//! Secret-shape redaction: [`redact_text`] replaces every run of text that has the shape of a
//! known credential with `[redacted:<label>]` and reports where it did so.
//!
//! The scan is hand-written and anchored on literal prefixes: no pattern engine, no entropy
//! guessing. A secret without one of the shapes below passes through unchanged, and an example
//! token with one of them is redacted.
//!
//! | [`Kind`] | Shape |
//! |---|---|
//! | `Github` | `ghp_`, `gho_`, `ghu_`, `ghs_` or `ghr_`, then 36 or more letters and digits |
//! | `GithubPat` | `github_pat_`, then 22 or more of letters, digits, `-` and `_` |
//! | `Anthropic` | `sk-ant-`, then 20 or more of letters, digits, `-` and `_` |
//! | `OpenAi` | `sk-` or `sk-proj-`, then 20 or more of letters, digits, `-` and `_` after the prefix |
//! | `Slack` | `xoxa-`, `xoxb-`, `xoxp-`, `xoxr-` or `xoxs-`, then 10 or more letters, digits and `-` |
//! | `Aws` | `AKIA` or `ASIA`, then 16 or more capital letters and digits |
//! | `Google` | `AIza`, then 35 or more of letters, digits, `-` and `_` |
//! | `Gitlab` | `glpat-`, then 20 or more of letters, digits, `-` and `_` |
//! | `Jwt` | three dot-separated runs of letters, digits, `-` and `_`: the first starts with `eyJ`, the other two are not empty |
//! | `PrivateKey` | a PEM `-----BEGIN … PRIVATE KEY-----` block through its matching `-----END` line, which is a line of its own |
//! | `Bearer` | the credential after the word `Bearer`, 20 or more of letters, digits and `-._~+/=` |
//! | `UrlPassword` | the password in `scheme://user:password@host`: the scheme starts with a letter and the host is not empty |
//!
//! Rules every shape shares:
//! - A shape takes the whole run that follows its prefix, so no tail of the secret stays outside
//!   the span. The number in the table is the minimum length of that run.
//! - A run stops before a `-----BEGIN ` delimiter, so a credential written straight against a
//!   PEM block cannot swallow the block's header.
//! - `sk-`, `sk-ant-` and `eyJ` start only after a byte that cannot continue such a run, and
//!   `AKIA`, `ASIA` and `Bearer` only after a byte that is not a letter or digit, so `task-…`
//!   and `MAKIA…` stay. The end of an earlier span counts as such a byte.
//! - A `PrivateKey` block without a matching `-----END` line of its own runs to the end of the
//!   input. Such a line starts after a line break and ends at a line break or the end of the
//!   input, and a line break is `\n` or `\r`.
//! - The `Bearer` and `UrlPassword` spans cover the credential only: `Bearer ` and the user
//!   name, host and path stay readable.

use std::ops::Range;

/// The shape of a redacted secret. [`Kind::label`] is its name inside the marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A GitHub personal, OAuth, user, server or refresh token.
    Github,
    /// A fine-grained GitHub personal access token.
    GithubPat,
    /// An Anthropic API key.
    Anthropic,
    /// An `OpenAI` API key.
    OpenAi,
    /// A Slack token.
    Slack,
    /// An AWS access key id.
    Aws,
    /// A Google API key.
    Google,
    /// A GitLab personal access token.
    Gitlab,
    /// A JSON web token.
    Jwt,
    /// A PEM private-key block.
    PrivateKey,
    /// The credential of a `Bearer` authorization header.
    Bearer,
    /// The password inside a URL's user information.
    UrlPassword,
}

impl Kind {
    /// The text between `[redacted:` and `]` in the replacement marker.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::GithubPat => "github-pat",
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Slack => "slack",
            Self::Aws => "aws",
            Self::Google => "google",
            Self::Gitlab => "gitlab",
            Self::Jwt => "jwt",
            Self::PrivateKey => "private-key",
            Self::Bearer => "bearer",
            Self::UrlPassword => "url-password",
        }
    }
}

/// Replaces each secret-shaped span of `input` with `[redacted:<label>]`.
///
/// Returns the new text and one `(kind, range)` per replacement. Each range is a byte range of
/// `input` on char boundaries, and the ranges are ascending, disjoint and non-empty. Text outside
/// the ranges is copied byte for byte, including literal marker-like text. The scan is leftmost
/// first and resumes at the end of each span. A span that starts after its anchor (the credential
/// of `Bearer`, the password of a URL) is held until the scan reaches its start, so a shape in
/// between, such as a token used as the user name, is found too.
#[must_use]
pub fn redact_text(input: &str) -> (String, Vec<(Kind, Range<usize>)>) {
    let bytes = input.as_bytes();
    let mut spans = Vec::new();
    let mut ahead: Option<(Kind, Range<usize>)> = None;
    let mut floor = 0;
    let mut at = 0;
    while at < bytes.len() {
        let found = match ahead.take_if(|(_, range)| range.start == at) {
            Some(due) => Some(due),
            None => shape_at(bytes, at, floor),
        };
        let Some((kind, range)) = found else {
            at += 1;
            continue;
        };
        let before_ahead = ahead
            .as_ref()
            .is_none_or(|(_, held)| range.end <= held.start);
        if range.start > at {
            if ahead.is_none() {
                ahead = Some((kind, range));
            }
            at += 1;
        } else if before_ahead {
            at = range.end;
            floor = range.end;
            spans.push((kind, range));
        } else {
            at += 1;
        }
    }
    let mut text = String::with_capacity(input.len());
    let mut cursor = 0;
    for (kind, range) in &spans {
        text.push_str(input.get(cursor..range.start).unwrap_or_default());
        text.push_str("[redacted:");
        text.push_str(kind.label());
        text.push(']');
        cursor = range.end;
    }
    text.push_str(input.get(cursor..).unwrap_or_default());
    (text, spans)
}

/// The first shape that starts at byte `at`, or `None`. `floor` is the end of the previous span.
fn shape_at(bytes: &[u8], at: usize, floor: usize) -> Option<(Kind, Range<usize>)> {
    match *bytes.get(at)? {
        b'g' => github(bytes, at)
            .map(|range| (Kind::Github, range))
            .or_else(|| github_pat(bytes, at).map(|range| (Kind::GithubPat, range)))
            .or_else(|| gitlab(bytes, at).map(|range| (Kind::Gitlab, range))),
        b's' => sk_key(bytes, at, floor),
        b'x' => slack(bytes, at).map(|range| (Kind::Slack, range)),
        b'A' => aws(bytes, at, floor)
            .map(|range| (Kind::Aws, range))
            .or_else(|| google(bytes, at).map(|range| (Kind::Google, range))),
        b'e' => jwt(bytes, at, floor).map(|range| (Kind::Jwt, range)),
        b'-' => private_key(bytes, at).map(|range| (Kind::PrivateKey, range)),
        b'B' | b'b' => bearer(bytes, at, floor).map(|range| (Kind::Bearer, range)),
        b':' => url_password(bytes, at).map(|range| (Kind::UrlPassword, range)),
        _ => None,
    }
}

fn github(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    let head = bytes.get(at..at + 4)?;
    let kind_ok = head.first() == Some(&b'g')
        && head.get(1) == Some(&b'h')
        && matches!(head.get(2), Some(b'p' | b'o' | b'u' | b's' | b'r'))
        && head.get(3) == Some(&b'_');
    if kind_ok {
        tail_run(bytes, at, 4, is_alnum, 36)
    } else {
        None
    }
}

fn github_pat(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    prefixed(bytes, at, b"github_pat_")?;
    tail_run(bytes, at, b"github_pat_".len(), is_word, 22)
}

fn gitlab(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    prefixed(bytes, at, b"glpat-")?;
    tail_run(bytes, at, b"glpat-".len(), is_word, 20)
}

/// `sk-ant-`, `sk-proj-` and plain `sk-` keys. Each prefix has its own 20-byte tail, so a short
/// `sk-ant-` or `sk-proj-` key is not read as a plain `sk-` key and stays unchanged.
fn sk_key(bytes: &[u8], at: usize, floor: usize) -> Option<(Kind, Range<usize>)> {
    if !clear_before(bytes, at, floor, is_word) {
        return None;
    }
    if prefixed(bytes, at, b"sk-ant-").is_some() {
        return tail_run(bytes, at, b"sk-ant-".len(), is_word, 20)
            .map(|range| (Kind::Anthropic, range));
    }
    if prefixed(bytes, at, b"sk-proj-").is_some() {
        return tail_run(bytes, at, b"sk-proj-".len(), is_word, 20)
            .map(|range| (Kind::OpenAi, range));
    }
    prefixed(bytes, at, b"sk-")?;
    tail_run(bytes, at, b"sk-".len(), is_word, 20).map(|range| (Kind::OpenAi, range))
}

fn slack(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    let head = bytes.get(at..at + 5)?;
    let head_ok = head.starts_with(b"xox")
        && matches!(head.get(3), Some(b'a' | b'b' | b'p' | b'r' | b's'))
        && head.get(4) == Some(&b'-');
    if head_ok {
        tail_run(bytes, at, 5, |c| is_alnum(c) || c == b'-', 10)
    } else {
        None
    }
}

fn aws(bytes: &[u8], at: usize, floor: usize) -> Option<Range<usize>> {
    let head = bytes.get(at..at + 4)?;
    if !(head == b"AKIA" || head == b"ASIA") || !clear_before(bytes, at, floor, is_alnum) {
        return None;
    }
    tail_run(
        bytes,
        at,
        4,
        |c| c.is_ascii_uppercase() || c.is_ascii_digit(),
        16,
    )
}

fn google(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    prefixed(bytes, at, b"AIza")?;
    tail_run(bytes, at, 4, is_word, 35)
}

/// The compact three-part shape: the first run starts with `eyJ`, and the second and third runs
/// are not empty.
fn jwt(bytes: &[u8], at: usize, floor: usize) -> Option<Range<usize>> {
    prefixed(bytes, at, b"eyJ")?;
    if !clear_before(bytes, at, floor, is_word) {
        return None;
    }
    let first = run_end(bytes, at + 3, is_word);
    let second_at = dot_then(bytes, first)?;
    let second = run_end(bytes, second_at, is_word);
    if second == second_at {
        return None;
    }
    let third_at = dot_then(bytes, second)?;
    let third = run_end(bytes, third_at, is_word);
    (third > third_at).then_some(at..third)
}

/// A PEM private-key block: from its header through its matching end line, or through the end
/// of the input when the end line is missing.
fn private_key(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    prefixed(bytes, at, PEM_BEGIN)?;
    let label_at = at + PEM_BEGIN.len();
    let rest = bytes.get(label_at..)?;
    let label_len = rest
        .iter()
        .take(PEM_LABEL_MAX + 1)
        .position(|&c| !(c.is_ascii_uppercase() || c == b' '))?;
    let label = rest.get(..label_len)?;
    if !label.ends_with(b"PRIVATE KEY") || !rest.get(label_len..)?.starts_with(PEM_DASHES) {
        return None;
    }
    let body_at = label_at + label_len + PEM_DASHES.len();
    let footer = [b"-----END ".as_slice(), label, PEM_DASHES].concat();
    let mut from = body_at;
    let end = loop {
        let Some(found) = bytes
            .get(from..)?
            .windows(footer.len())
            .position(|window| window == footer.as_slice())
        else {
            break bytes.len();
        };
        let start = from + found;
        let stop = start + footer.len();
        let line_start = start
            .checked_sub(1)
            .and_then(|prev| bytes.get(prev))
            .is_some_and(|&c| is_line_break(c));
        if line_start && bytes.get(stop).is_none_or(|&c| is_line_break(c)) {
            break stop;
        }
        from = start + 1;
    };
    Some(at..end)
}

/// The credential after `Bearer` and its blanks.
fn bearer(bytes: &[u8], at: usize, floor: usize) -> Option<Range<usize>> {
    let word = bytes.get(at..at + 6)?;
    if !word.eq_ignore_ascii_case(b"bearer") || !clear_before(bytes, at, floor, is_alnum) {
        return None;
    }
    let blanks = bytes
        .get(at + 6..)?
        .iter()
        .take_while(|&&c| c == b' ' || c == b'\t')
        .count();
    if blanks == 0 {
        return None;
    }
    let start = at + 6 + blanks;
    let end = run_end(bytes, start, |c| is_alnum(c) || b"-._~+/=".contains(&c));
    (end - start >= 20).then_some(start..end)
}

/// The password of `scheme://user:password@host`, found at the `:` of `://`.
fn url_password(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    prefixed(bytes, at, b"://")?;
    if !scheme_before(bytes, at) {
        return None;
    }
    let start = at + 3;
    let authority = bytes.get(start..)?;
    let length = authority
        .iter()
        .position(|&c| c <= b' ' || c == 0x7f || b"/?#\"'<>`".contains(&c))
        .unwrap_or(authority.len());
    let authority = authority.get(..length)?;
    let at_sign = authority.iter().rposition(|&c| c == b'@')?;
    let userinfo = authority.get(..at_sign)?;
    let colon = userinfo.iter().position(|&c| c == b':')?;
    let host_follows = at_sign + 1 < authority.len();
    (colon + 1 < at_sign && host_follows).then_some(start + colon + 1..start + at_sign)
}

/// Whether the run of scheme bytes (letters, digits, `+`, `-`, `.`) that ends at `end` is not empty
/// and starts with a letter.
fn scheme_before(bytes: &[u8], end: usize) -> bool {
    let scheme = |c: u8| is_alnum(c) || matches!(c, b'+' | b'-' | b'.');
    let mut start = end;
    while let Some(prev) = start.checked_sub(1) {
        if !bytes.get(prev).is_some_and(|&c| scheme(c)) {
            break;
        }
        start = prev;
    }
    start < end && bytes.get(start).is_some_and(u8::is_ascii_alphabetic)
}

const PEM_BEGIN: &[u8] = b"-----BEGIN ";
const PEM_DASHES: &[u8] = b"-----";
/// The longest label between `-----BEGIN ` and `-----` that a header carries.
const PEM_LABEL_MAX: usize = 64;

/// `at + prefix.len()` when `bytes` has `prefix` at `at`.
fn prefixed(bytes: &[u8], at: usize, prefix: &[u8]) -> Option<usize> {
    bytes
        .get(at..)?
        .starts_with(prefix)
        .then_some(at + prefix.len())
}

/// The span `at..end` when the run of `class` bytes after `head` bytes is `min` or longer.
fn tail_run(
    bytes: &[u8],
    at: usize,
    head: usize,
    class: fn(u8) -> bool,
    min: usize,
) -> Option<Range<usize>> {
    let start = at + head;
    let end = run_end(bytes, start, class);
    (end - start >= min).then_some(at..end)
}

/// The end of the run of `class` bytes that starts at `from`. A run stops before `-----BEGIN `.
fn run_end(bytes: &[u8], from: usize, class: impl Fn(u8) -> bool) -> usize {
    let mut end = from;
    while let Some(&c) = bytes.get(end) {
        let delimiter = c == b'-'
            && bytes
                .get(end..)
                .is_some_and(|rest| rest.starts_with(PEM_BEGIN));
        if !class(c) || delimiter {
            break;
        }
        end += 1;
    }
    end
}

/// The index after a `.` at `at`.
fn dot_then(bytes: &[u8], at: usize) -> Option<usize> {
    (bytes.get(at) == Some(&b'.')).then_some(at + 1)
}

/// Whether a shape may start at `at`: it starts the input or follows the previous span, or the
/// byte before it is not in `blocking`.
fn clear_before(bytes: &[u8], at: usize, floor: usize, blocking: fn(u8) -> bool) -> bool {
    at == floor
        || at
            .checked_sub(1)
            .and_then(|prev| bytes.get(prev))
            .is_none_or(|&c| !blocking(c))
}

fn is_line_break(c: u8) -> bool {
    c == b'\n' || c == b'\r'
}

fn is_alnum(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/// Letters, digits, `-` and `_`.
fn is_word(c: u8) -> bool {
    is_alnum(c) || c == b'-' || c == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(input: &str) -> Vec<&'static str> {
        redact_text(input)
            .1
            .iter()
            .map(|(kind, _)| kind.label())
            .collect()
    }

    #[test]
    fn labels_are_distinct_and_inside_the_marker_alphabet() {
        let kinds = [
            Kind::Github,
            Kind::GithubPat,
            Kind::Anthropic,
            Kind::OpenAi,
            Kind::Slack,
            Kind::Aws,
            Kind::Google,
            Kind::Gitlab,
            Kind::Jwt,
            Kind::PrivateKey,
            Kind::Bearer,
            Kind::UrlPassword,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for kind in kinds {
            let label = kind.label();
            assert!(!label.is_empty());
            assert!(
                label.bytes().all(|c| c.is_ascii_lowercase() || c == b'-'),
                "{label}"
            );
            assert!(seen.insert(label), "{label} repeats");
        }
    }

    #[test]
    fn a_pem_header_written_against_a_credential_is_not_swallowed() {
        let secret = format!("sk-{}", "a".repeat(20));
        let block = "-----BEGIN PRIVATE KEY-----\nZHVtbXk=\n-----END PRIVATE KEY-----";
        let input = format!("{secret}{block}");
        let (text, spans) = redact_text(&input);
        assert_eq!(
            spans,
            [
                (Kind::OpenAi, 0..secret.len()),
                (Kind::PrivateKey, secret.len()..input.len()),
            ]
        );
        assert_eq!(text, "[redacted:openai][redacted:private-key]");
    }

    #[test]
    fn a_credential_too_short_before_a_pem_header_leaves_the_block_redacted() {
        let input = format!("glpat-{}-----BEGIN PRIVATE KEY-----", "a".repeat(5));
        assert_eq!(labels(&input), ["private-key"]);
    }

    #[test]
    fn the_end_line_of_a_block_must_carry_the_same_label() {
        let input = "-----BEGIN RSA PRIVATE KEY-----\nx\n-----END PRIVATE KEY-----\nstill secret";
        let (text, spans) = redact_text(input);
        assert_eq!(spans, [(Kind::PrivateKey, 0..input.len())]);
        assert_eq!(text, "[redacted:private-key]");
    }

    #[test]
    fn a_header_that_is_not_a_private_key_header_stays() {
        for input in [
            "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----",
            "-----BEGIN PRIVATE KEY----",
            "-----BEGIN private key-----\nabc",
            "-----BEGIN PRIVATE KEY",
        ] {
            assert_eq!(labels(input), Vec::<&str>::new(), "{input}");
        }
        let long = format!("-----BEGIN {}PRIVATE KEY-----", "A".repeat(70));
        assert_eq!(labels(&long), Vec::<&str>::new());
    }

    #[test]
    fn a_short_anthropic_key_is_not_read_as_a_plain_key() {
        assert_eq!(
            labels(&format!("sk-ant-{}", "a".repeat(19))),
            Vec::<&str>::new()
        );
        assert_eq!(labels(&format!("sk-ant-{}", "a".repeat(20))), ["anthropic"]);
    }

    #[test]
    fn near_misses_on_the_minimum_stay_and_the_minimum_itself_goes() {
        let cases: [(&str, &str, usize, &str); 10] = [
            ("github_pat_", "a", 22, "github-pat"),
            ("glpat-", "a", 20, "gitlab"),
            ("AIza", "a", 35, "google"),
            ("xoxb-", "1", 10, "slack"),
            ("AKIA", "A", 16, "aws"),
            ("ghp_", "a", 36, "github"),
            ("sk-", "a", 20, "openai"),
            ("sk-proj-", "a", 20, "openai"),
            ("sk-ant-", "a", 20, "anthropic"),
            ("Bearer ", "a", 20, "bearer"),
        ];
        for (prefix, fill, min, label) in cases {
            let short = format!("{prefix}{}", fill.repeat(min - 1));
            assert_eq!(labels(&short), Vec::<&str>::new(), "{short}");
            let exact = format!("{prefix}{}", fill.repeat(min));
            assert_eq!(labels(&exact), [label], "{exact}");
        }
    }

    #[test]
    fn jwt_needs_three_non_empty_runs() {
        for input in [
            "eyJabc.def",
            "eyJabc.def.",
            "eyJabc..ghi",
            "xeyJabc.def.ghi",
        ] {
            assert_eq!(labels(input), Vec::<&str>::new(), "{input}");
        }
        assert_eq!(labels("eyJ.d.e"), ["jwt"]);
    }

    #[test]
    fn aws_and_bearer_do_not_start_inside_a_word() {
        assert_eq!(
            labels(&format!("MAKIA{}", "A".repeat(16))),
            Vec::<&str>::new()
        );
        assert_eq!(
            labels(&format!("xBearer {}", "a".repeat(20))),
            Vec::<&str>::new()
        );
        assert_eq!(
            labels(&format!("Bearer{}", "a".repeat(20))),
            Vec::<&str>::new()
        );
        assert_eq!(labels(&format!("KEY_AKIA{}", "A".repeat(16))), ["aws"]);
    }

    #[test]
    fn the_url_password_runs_to_the_last_at_sign_of_the_authority() {
        let input = "x https://u:p@ss:w@host/p@th";
        let (text, spans) = redact_text(input);
        assert_eq!(text, "x https://u:[redacted:url-password]@host/p@th");
        assert_eq!(spans, [(Kind::UrlPassword, 12..18)]);
    }

    #[test]
    fn a_url_password_may_hold_non_ascii_and_the_ranges_stay_on_char_boundaries() {
        let input = "ssh://név:pässwörd@host\u{1F512}";
        let (text, spans) = redact_text(input);
        assert_eq!(text, "ssh://név:[redacted:url-password]@host\u{1F512}");
        for (_, range) in &spans {
            assert!(input.is_char_boundary(range.start) && input.is_char_boundary(range.end));
        }
    }

    #[test]
    fn urls_without_a_scheme_a_password_or_a_user_part_stay() {
        for input in [
            "://u:p@h/",
            "https://u@h:8080/",
            "https://h:8080/a@b",
            "https://u:@h/",
            "https://u:p h/",
        ] {
            assert_eq!(labels(input), Vec::<&str>::new(), "{input}");
        }
        assert_eq!(labels("https://:p@h/"), ["url-password"]);
    }

    #[test]
    fn a_credential_inside_a_url_password_is_one_url_password_span() {
        let token = format!("ghp_{}", "a".repeat(36));
        assert_eq!(labels(&format!("https://u:{token}@h")), ["url-password"]);
    }

    #[test]
    fn a_token_used_as_the_url_user_name_is_redacted_too() {
        let token = format!("ghp_{}", "a".repeat(36));
        let input = format!("https://{token}:x-oauth-basic@github.test/");
        let (text, spans) = redact_text(&input);
        assert_eq!(
            text,
            "https://[redacted:github]:[redacted:url-password]@github.test/"
        );
        assert_eq!(spans, [(Kind::Github, 8..48), (Kind::UrlPassword, 49..62)]);
    }

    #[test]
    fn the_authority_of_a_url_ends_at_each_delimiter_and_only_those() {
        for end in [
            "/", "?", "#", "\"", "'", "<", ">", "`", " ", "\t", "\n", "\u{1}", "\u{7f}",
        ] {
            let input = format!("https://u:p{end}@h");
            assert_eq!(labels(&input), Vec::<&str>::new(), "{input:?}");
        }
        for inside in ["!", "(", ")", "[", "]", "%", "{", "~", "\u{80}", "é"] {
            let input = format!("https://u:p{inside}@h");
            assert_eq!(labels(&input), ["url-password"], "{input:?}");
        }
    }

    #[test]
    fn a_pem_label_holds_at_most_sixty_four_bytes() {
        let header = |label: &str| format!("-----BEGIN {label}-----\nbody");
        let sixty_four = format!("{}PRIVATE KEY", "A".repeat(53));
        assert_eq!(labels(&header(&sixty_four)), ["private-key"]);
        let sixty_five = format!("{}PRIVATE KEY", "A".repeat(54));
        assert_eq!(labels(&header(&sixty_five)), Vec::<&str>::new());
        assert_eq!(labels(&header("PRIVATE KEY")), ["private-key"]);
        assert_eq!(labels(&header(" PRIVATE KEY")), ["private-key"]);
    }

    #[test]
    fn a_prefix_after_a_word_byte_starts_no_shape_where_the_plan_says_so() {
        let sk = format!("sk-{}", "a".repeat(20));
        let aws = format!("AKIA{}", "A".repeat(16));
        let jwt = "eyJa.b.c";
        for before in ["a", "Z", "5", "-", "_"] {
            assert_eq!(
                labels(&format!("{before}{sk}")),
                Vec::<&str>::new(),
                "{before}"
            );
            assert_eq!(
                labels(&format!("{before}{jwt}")),
                Vec::<&str>::new(),
                "{before}"
            );
        }
        for before in ["a", "Z", "5"] {
            assert_eq!(
                labels(&format!("{before}{aws}")),
                Vec::<&str>::new(),
                "{before}"
            );
        }
        for before in [" ", ".", "/", "=", "\"", "é", "\n"] {
            assert_eq!(labels(&format!("{before}{sk}")), ["openai"], "{before}");
        }
        for before in ["-", "_", ".", " "] {
            assert_eq!(labels(&format!("{before}{aws}")), ["aws"], "{before}");
        }
        for before in [".", "/", " "] {
            assert_eq!(labels(&format!("{before}{jwt}")), ["jwt"], "{before}");
        }
    }

    #[test]
    fn prefixes_with_no_left_boundary_match_inside_a_word() {
        let github = format!("ghp_{}", "a".repeat(36));
        let google = format!("AIza{}", "a".repeat(35));
        let gitlab = format!("glpat-{}", "a".repeat(20));
        let slack = format!("xoxb-{}", "1".repeat(10));
        for secret in [github, google, gitlab, slack] {
            assert_eq!(labels(&format!("x{secret}")).len(), 1, "{secret}");
        }
    }

    #[test]
    fn a_run_ends_at_the_first_byte_outside_its_class() {
        let (text, spans) = redact_text(&format!("ghp_{}_tail", "a".repeat(36)));
        assert_eq!(spans, [(Kind::Github, 0..40)]);
        assert_eq!(text, "[redacted:github]_tail");
        let (_, spans) = redact_text(&format!("ghp_{}", "a".repeat(50)));
        assert_eq!(spans, [(Kind::Github, 0..54)]);
        let (_, spans) = redact_text(&format!("xoxs-{}.tail", "1-".repeat(8)));
        assert_eq!(spans, [(Kind::Slack, 0..21)]);
        let (_, spans) = redact_text(&format!("AKIA{}a", "A1".repeat(10)));
        assert_eq!(spans, [(Kind::Aws, 0..24)]);
    }

    #[test]
    fn bearer_blanks_are_spaces_and_tabs_only() {
        let credential = "a".repeat(20);
        for blanks in [" ", "\t", "   ", " \t "] {
            let input = format!("Bearer{blanks}{credential}");
            let start = 6 + blanks.len();
            assert_eq!(redact_text(&input).1, [(Kind::Bearer, start..start + 20)]);
        }
        for blanks in ["\n", "\r", "\u{b}", ": ", "\u{a0}"] {
            let input = format!("Bearer{blanks}{credential}");
            assert_eq!(labels(&input), Vec::<&str>::new(), "{input:?}");
        }
        assert_eq!(labels(&format!("BEARER {credential}")), ["bearer"]);
        assert_eq!(labels(&format!("bEaReR {credential}")), ["bearer"]);
    }

    #[test]
    fn a_url_scheme_starts_with_a_letter_and_the_host_is_not_empty() {
        for scheme in ["https", "s3", "a+", "a-", "a.", "A", "x-1.y+z"] {
            let input = format!("{scheme}://u:p@h");
            assert_eq!(labels(&input), ["url-password"], "{input}");
        }
        for scheme in ["", "123", "-", "1http", "+a", ".a", "9-a"] {
            let input = format!("{scheme}://u:p@h");
            assert_eq!(labels(&input), Vec::<&str>::new(), "{input:?}");
        }
        for before in [" ", "_", "(", "\"", "é"] {
            let input = format!("{before}://u:p@h");
            assert_eq!(labels(&input), Vec::<&str>::new(), "{input:?}");
        }
        for input in [
            "https://u:p@",
            "https://u:p@/x",
            "https://u:p@ h",
            "https://u:p@?q",
        ] {
            assert_eq!(labels(input), Vec::<&str>::new(), "{input:?}");
        }
        assert_eq!(labels("https://u:p@h"), ["url-password"]);
    }

    #[test]
    fn the_end_line_of_a_block_is_a_line_of_its_own() {
        let footer = "-----END PRIVATE KEY-----";
        let head = "-----BEGIN PRIVATE KEY-----\nabc\n";
        let prose = format!("{head}see {footer} in the docs\nSECRET\nmore");
        assert_eq!(redact_text(&prose).1, [(Kind::PrivateKey, 0..prose.len())]);
        let trailing = format!("{head}{footer}x\nSECRET");
        assert_eq!(
            redact_text(&trailing).1,
            [(Kind::PrivateKey, 0..trailing.len())]
        );
        let both = format!("{head}see {footer} here\ndef\n{footer}\nafter");
        let closed = both.len() - "\nafter".len();
        assert_eq!(redact_text(&both).1, [(Kind::PrivateKey, 0..closed)]);
        for breaks in ["\n", "\r\n", "\r"] {
            let input =
                format!("-----BEGIN PRIVATE KEY-----{breaks}abc{breaks}{footer}{breaks}after");
            let closed = input.len() - breaks.len() - "after".len();
            assert_eq!(
                redact_text(&input).1,
                [(Kind::PrivateKey, 0..closed)],
                "{breaks:?}"
            );
        }
        let joined = format!("-----BEGIN PRIVATE KEY-----{footer}\nrest");
        assert_eq!(
            redact_text(&joined).1,
            [(Kind::PrivateKey, 0..joined.len())]
        );
    }

    /// The structural promises of [`redact_text`] over one input: ascending, disjoint,
    /// non-empty spans on char boundaries, the text rebuilt from the input and the markers, and a
    /// second pass that finds nothing wholly inside the text the first pass kept.
    fn check_invariants(input: &str) {
        let (text, spans) = redact_text(input);
        let mut expected = String::new();
        let mut kept = Vec::new();
        let mut cursor = 0;
        for (kind, range) in &spans {
            assert!(
                range.start < range.end && range.start >= cursor,
                "{input:?}: {spans:?}"
            );
            assert!(
                input.is_char_boundary(range.start) && input.is_char_boundary(range.end),
                "{input:?}: {spans:?}"
            );
            let unchanged = &input[cursor..range.start];
            kept.push(expected.len()..expected.len() + unchanged.len());
            expected.push_str(unchanged);
            expected.push_str("[redacted:");
            expected.push_str(kind.label());
            expected.push(']');
            cursor = range.end;
        }
        let tail = &input[cursor..];
        kept.push(expected.len()..expected.len() + tail.len());
        expected.push_str(tail);
        assert_eq!(text, expected, "{input:?}");
        for (kind, range) in redact_text(&text).1 {
            assert!(
                !kept
                    .iter()
                    .any(|part| part.start <= range.start && range.end <= part.end),
                "{input:?}: a {} shape survived at {range:?} in {text:?}",
                kind.label()
            );
        }
    }

    #[test]
    fn generated_texts_keep_the_span_invariants() {
        let long = "a".repeat(20);
        let upper = "A1".repeat(10);
        let pieces = [
            "sk-",
            "sk-ant-",
            "sk-proj-",
            "ghp_",
            "github_pat_",
            "glpat-",
            "AIza",
            "xoxb-",
            "AKIA",
            "eyJ",
            ".",
            "-",
            "_",
            ":",
            "@",
            "//",
            "://",
            "https",
            "u:p@h",
            "Bearer ",
            "bearer\t",
            "-----BEGIN PRIVATE KEY-----",
            "-----END PRIVATE KEY-----",
            "-----BEGIN ",
            "\n",
            "\r\n",
            " ",
            "é",
            "[redacted:x]",
            "0123456789",
            long.as_str(),
            upper.as_str(),
        ];
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |bound: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(state >> 33).unwrap_or(0) % bound
        };
        for _ in 0..20_000 {
            let mut input = String::new();
            for _ in 0..=next(24) {
                input.push_str(pieces[next(pieces.len())]);
            }
            check_invariants(&input);
        }
    }

    #[test]
    fn github_and_slack_heads_need_every_byte_of_their_prefix() {
        let github_tail = "a".repeat(36);
        for head in ["gxp_", "ghx_", "ghp-", "ghp ", "gHp_", "ghP_"] {
            let input = format!("{head}{github_tail}");
            assert_eq!(labels(&input), Vec::<&str>::new(), "{head}");
        }
        let slack_tail = "1".repeat(10);
        for head in ["xoxz-", "xyyb-", "xxxb-", "xoxb_", "xoxb ", "xOxb-"] {
            let input = format!("{head}{slack_tail}");
            assert_eq!(labels(&input), Vec::<&str>::new(), "{head}");
        }
    }

    #[test]
    fn a_jwt_is_found_wherever_it_starts() {
        for pad in [1, 2, 5, 10, 40] {
            let input = format!("{}eyJ.d.e", " ".repeat(pad));
            assert_eq!(redact_text(&input).1, [(Kind::Jwt, pad..pad + 7)], "{pad}");
        }
    }

    #[test]
    fn bearer_stops_at_the_first_byte_outside_its_alphabet() {
        let input = format!("Bearer {}, next", "a".repeat(20));
        let (text, spans) = redact_text(&input);
        assert_eq!(spans, [(Kind::Bearer, 7..27)]);
        assert_eq!(text, "Bearer [redacted:bearer], next");
    }

    #[test]
    fn empty_and_marker_only_input_pass_through() {
        for input in ["", "[redacted:openai]", "\u{0}\u{7f}é"] {
            let (text, spans) = redact_text(input);
            assert_eq!(text, input);
            assert!(spans.is_empty());
        }
    }

    #[test]
    fn many_secrets_each_get_their_own_ascending_span() {
        let aws = format!("AKIA{}", "B".repeat(16));
        let input = format!("{aws} and {aws}\n{aws}");
        let spans = redact_text(&input).1;
        assert_eq!(spans.len(), 3);
        assert!(
            spans
                .windows(2)
                .all(|pair| pair[0].1.end <= pair[1].1.start)
        );
    }
}
