//! The vendor's account-block lines: a refusal that no reset frees.
//!
//! A usage limit waits for a reset. These lines say the account itself is
//! refused: a payment that failed verification, a subscription whose monthly
//! quota is spent. Each is MEASURED per tool, read exactly like a usage-limit
//! notice (the pane's own row, never a quote of it) and booked in the same
//! latch, so the list cell, the attention rank and the auto reseat treat it as
//! a usage limit. The one difference is the KIND ([`Kind`]), which rides in the
//! notice's own words ([`kind_of`]) and decides the cell and the scope block.
//!
//! No measurement, no line: another error text, another indent or an unmeasured
//! footer reads as nothing, which is today's behaviour and never a false block.
//!
//! - muse 1.4.25 draws a refused turn as `◆ model failed: API error 402
//!   [request_id=…]: Billing verification failed. Billing Issue` at column 0.
//!   The row is the reconstructed frame (`tests/fixtures/vendorblock/README.md`):
//!   the 402 did not reproduce on the probe day.
//! - opencode 2.4.1 draws a Copilot quota refusal as a TRANSCRIPT row, five
//!   spaces and `Error: You have exceeded your monthly quota`, MEASURED. It books
//!   only as the newest transcript row (below it only blank rows, the measured
//!   `     <agent> · <model> · <n>ms` row and the composer), because a quote of
//!   the same words in a peek, a log or tool output is the agent's own text and
//!   no refusal of its account.

use super::{Banner, Lead, NOTICE_CELLS, newest_row};
use crate::vendorblock::Kind;

/// muse's refused-turn head, a START of the cell's text.
const MUSE_HEAD: &str = "model failed: API error 402";

/// The words that make a muse 402 a BILLING refusal and not another 402.
const MUSE_BILLING: &str = "Billing verification failed";

/// muse draws the row at column 0 with `◆`, in the same window as its usage
/// banner (`MUSE_BANNER`); the 402 row shares the 429 row's measured shape.
const MUSE_BLOCK: Banner = Banner {
    lead: Lead::Marks(&["◆"]),
    window: 16,
    phrases: &[MUSE_HEAD],
};

/// The opencode row's text after its five-space indent.
const OPENCODE_ROW: &str = "Error: You have exceeded your monthly quota";

/// The indent of every opencode transcript row measured.
const OPENCODE_INDENT: &str = "     ";

/// The blanks between opencode's transcript column and its sidebar, measured at
/// 19 widths: the sidebar starts `L + 4` columns in, `L` being the composer
/// rule's length, and a pane of 120 columns or fewer has none.
const SIDEBAR_GAP: usize = 4;

/// The notice a block frame shows for `agent_bin`, or `None`: the cell as the
/// vendor drew it, its `[request_id=…]` scrubbed and clipped like a usage
/// notice.
pub(super) fn notice(buf: &str, agent_bin: &str) -> Option<String> {
    match agent_bin {
        "muse" => muse(buf),
        "opencode" => opencode(buf).map(|found| found.notice),
        _ => None,
    }
}

/// The drawn rows of the cell [`notice`] reads, for the stop proof.
pub(super) fn rows<'a>(buf: &'a str, agent_bin: &str) -> Option<Vec<&'a str>> {
    match agent_bin {
        "muse" => {
            let (window, at, _) = newest_row(buf, &MUSE_BLOCK)?;
            muse(buf)?;
            Some(
                window[at..]
                    .iter()
                    .copied()
                    .take_while(|row| !row.trim().is_empty())
                    .collect(),
            )
        }
        "opencode" => {
            let lines: Vec<&str> = buf.lines().collect();
            let found = opencode(buf)?;
            Some(vec![lines[found.row], lines[found.footer]])
        }
        _ => None,
    }
}

/// What a vendor refused, and the binary name of the harness that drew the line
/// (`muse`, `opencode`), read back out of a notice [`notice`] produced, or `None`
/// for any other notice (a usage limit's, a menu's). The binary name is the
/// adapter's key: its `ToolKind` is `ToolKind::from_binary_name`'s to say.
#[must_use]
pub fn classify(notice: &str) -> Option<(Kind, &'static str)> {
    if notice.starts_with(MUSE_HEAD) && notice.contains(MUSE_BILLING) {
        Some((Kind::Billing, "muse"))
    } else if notice.starts_with(OPENCODE_ROW) {
        Some((Kind::Account, "opencode"))
    } else {
        None
    }
}

/// What a vendor refused, read back out of a notice [`notice`] produced.
#[must_use]
pub fn kind_of(notice: &str) -> Option<Kind> {
    classify(notice).map(|(kind, _)| kind)
}

/// The vendor's own reason inside a block notice: the words after the head, with
/// no label and no request id. The whole notice when it is no block's.
#[must_use]
pub fn reason_of(notice: &str) -> &str {
    match kind_of(notice) {
        Some(Kind::Billing) => notice[MUSE_HEAD.len()..]
            .trim_start_matches([':', ' '])
            .trim(),
        Some(Kind::Account) => notice["Error: ".len()..].trim(),
        None => notice,
    }
}

/// The scrubbed, clipped cell of muse's newest billing 402 row.
fn muse(buf: &str) -> Option<String> {
    let (window, at, text) = newest_row(buf, &MUSE_BLOCK)?;
    // muse wraps a long cell onto rows indented two spaces.
    let cell = std::iter::once(text.trim())
        .chain(
            window[at + 1..]
                .iter()
                .take_while(|row| row.starts_with("  ") && !row.trim().is_empty())
                .map(|row| row.trim()),
        )
        .collect::<Vec<_>>()
        .join(" ");
    cell.contains(MUSE_BILLING)
        .then(|| crate::event_text::display_column(&scrub(&cell), NOTICE_CELLS))
}

/// `cell` without its `[request_id=…]`: a vendor's request id names one request
/// to the vendor and nothing to a reader of the journal.
fn scrub(cell: &str) -> String {
    let Some(start) = cell.find("[request_id=") else {
        return cell.to_owned();
    };
    let mut out = cell[..start].trim_end().to_owned();
    if let Some(len) = cell[start..].find(']') {
        out.push_str(&cell[start + len + 1..]);
    }
    out
}

/// A found opencode refusal: where its row and footer sit in the capture, and
/// its notice.
struct Found {
    row: usize,
    footer: usize,
    notice: String,
}

/// opencode's newest transcript row, when it is the measured refusal.
fn opencode(buf: &str) -> Option<Found> {
    let rows: Vec<&str> = buf.lines().collect();
    let rule = rows.iter().rposition(|row| rule_length(row).is_some())?;
    let column = rule_length(rows[rule])? + SIDEBAR_GAP;
    let blank = |row: &str| cell(row, column).is_some_and(str::is_empty);
    let want = format!("{OPENCODE_INDENT}{OPENCODE_ROW}");
    let at = rows[..rule]
        .iter()
        .rposition(|row| cell(row, column) == Some(want.as_str()))?;
    let mut next = at + 1;
    while next < rule && blank(rows[next]) {
        next += 1;
    }
    let footer = next;
    if !cell(rows.get(footer)?, column).is_some_and(is_footer) {
        return None;
    }
    next += 1;
    while next < rule && blank(rows[next]) {
        next += 1;
    }
    // Only the composer's own block may stand between the footer and its rule:
    // a user echo that follows the refusal draws the same `┃` bar but is closed
    // by a blank row, never by the rule.
    if next >= rule || !rows[next..rule].iter().all(|row| row.starts_with("  ┃")) {
        return None;
    }
    // The composer's own block ends in its status row (`  ┃  Build · <model> …`):
    // a user echo that sits right above the rule has none.
    if !rows[rule - 1].contains(" · ") {
        return None;
    }
    // Under the rule only the one key-hint row the composer draws.
    if rows[rule + 1..].iter().filter(|row| !blank(row)).count() > 1 {
        return None;
    }
    Some(Found {
        row: at,
        footer,
        notice: OPENCODE_ROW.to_owned(),
    })
}

/// The transcript column of `row` at a pane whose sidebar starts at `column`:
/// the row without its sidebar text and trailing blanks, or `None` when the row
/// has text at or past `column` that is not a sidebar that starts exactly there.
fn cell(row: &str, column: usize) -> Option<&str> {
    let Some((at, _)) = row.char_indices().nth(column) else {
        return Some(row.trim_end());
    };
    let (left, right) = row.split_at(at);
    if right.trim().is_empty() {
        return Some(left.trim_end());
    }
    (left.ends_with(' ') && !right.starts_with(' ')).then(|| left.trim_end())
}

/// The length in columns of opencode's composer rule row (`  ╹▀▀…`), which
/// carries the pane's sidebar column, or `None` when `row` is no such rule.
fn rule_length(row: &str) -> Option<usize> {
    let rest = row.strip_prefix("  ╹")?;
    let run = rest.chars().take_while(|ch| *ch == '▀').count();
    if run == 0 {
        return None;
    }
    let length = 3 + run;
    let tail: String = rest.chars().skip(run).collect();
    (tail.trim().is_empty()
        || cell(row, length + SIDEBAR_GAP).is_some_and(|left| left.chars().count() == length))
    .then_some(length)
}

/// opencode's measured footer under a failed turn: five spaces, the agent's
/// name, the model and the turn's time in milliseconds, joined by ` · `. A time
/// in seconds or any other grammar is unmeasured and reads as no footer.
fn is_footer(cell: &str) -> bool {
    let Some(rest) = cell.strip_prefix(OPENCODE_INDENT) else {
        return false;
    };
    let mut parts = rest.split(" · ");
    let (Some(agent), Some(model), Some(took), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    !agent.is_empty()
        && agent
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        && !model.trim().is_empty()
        && model == model.trim()
        && !model.chars().any(char::is_control)
        && took
            .strip_suffix("ms")
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{cell, is_footer, kind_of, notice, reason_of, rows, rule_length, scrub};
    use crate::vendorblock::Kind;

    #[test]
    fn a_request_id_is_scrubbed_from_the_cell() {
        assert_eq!(
            scrub("model failed: API error 402 [request_id=abc-123]: Billing verification failed."),
            "model failed: API error 402: Billing verification failed."
        );
        assert_eq!(scrub("no id here"), "no id here");
        assert_eq!(scrub("head [request_id=cut"), "head");
    }

    #[test]
    fn a_notice_names_its_kind_and_its_reason() {
        let muse = "model failed: API error 402: Billing verification failed. Billing Issue";
        assert_eq!(kind_of(muse), Some(Kind::Billing));
        assert_eq!(
            reason_of(muse),
            "Billing verification failed. Billing Issue"
        );
        let copilot = "Error: You have exceeded your monthly quota";
        assert_eq!(kind_of(copilot), Some(Kind::Account));
        assert_eq!(reason_of(copilot), "You have exceeded your monthly quota");
        for usage in [
            "You've hit your session limit",
            "model failed: API error 429: Subscription quota exhausted.",
            "model failed: API error 402: Payment declined",
        ] {
            assert_eq!(kind_of(usage), None, "{usage}");
            assert_eq!(reason_of(usage), usage);
        }
    }

    #[test]
    fn the_sidebar_starts_exactly_at_its_column_or_the_row_is_no_cell() {
        let pad = |n: usize| " ".repeat(n);
        let row = format!("     Error: x{}side", pad(10));
        // `Error: x` ends at column 13; the sidebar starts at column 23.
        assert_eq!(cell(&row, 23), Some("     Error: x"));
        assert_eq!(
            cell(&row, 22),
            None,
            "text at or past the column, one early"
        );
        assert_eq!(cell(&row, 24), None, "one late");
        assert_eq!(cell("short", 23), Some("short"));
        assert_eq!(cell("", 23), Some(""));
        assert_eq!(
            cell(&format!("{}side", pad(23)), 23),
            Some(""),
            "sidebar only"
        );
    }

    #[test]
    fn the_rule_carries_the_pane_s_sidebar_column() {
        assert_eq!(rule_length("  ╹▀▀▀▀"), Some(7));
        assert_eq!(rule_length("  ╹"), None);
        assert_eq!(rule_length("  ┃▀▀▀"), None);
        assert_eq!(rule_length("╹▀▀▀"), None);
        let with_sidebar = format!("  ╹▀▀▀▀{}side", " ".repeat(4));
        assert_eq!(rule_length(&with_sidebar), Some(7));
        let off = format!("  ╹▀▀▀▀{}side", " ".repeat(5));
        assert_eq!(rule_length(&off), None);
    }

    #[test]
    fn a_muse_cell_ends_at_the_first_row_that_is_not_its_indented_continuation() {
        let frame = "\u{25c6} model failed: API error 402 [request_id=abc]: Billing verification failed.\n  Billing Issue\nnext output row\n";
        assert_eq!(
            notice(frame, "muse").as_deref(),
            Some("model failed: API error 402: Billing verification failed. Billing Issue")
        );
        assert_eq!(notice(frame, "opencode"), None);
        assert_eq!(notice(frame, "claude"), None);
    }

    #[test]
    fn the_opencode_stop_proof_rows_are_the_refusal_row_and_its_footer() {
        let error = "     Error: You have exceeded your monthly quota";
        let footer = "     Build \u{b7} GPT-6.1 Sol \u{b7} 692ms";
        let rule = format!("  \u{2579}{}", "\u{2580}".repeat(60));
        let frame = [
            error,
            "",
            footer,
            "",
            "  \u{2503}  Build \u{b7} GPT-6.1 Sol",
            rule.as_str(),
            "                         ctrl+p commands",
        ]
        .join("\n");
        assert_eq!(rows(&frame, "opencode"), Some(vec![error, footer]));
        assert_eq!(
            notice(&frame, "opencode").as_deref(),
            Some("Error: You have exceeded your monthly quota")
        );
        assert_eq!(rows(&frame, "claude"), None);
        assert_eq!(notice(&frame, "muse"), None);
    }

    #[test]
    fn only_the_measured_footer_grammar_is_a_footer() {
        assert!(is_footer("     Build · GPT-6.1 Sol · 692ms"));
        assert!(is_footer("     Plan · GPT-6.1 Sol · 3ms"));
        for no in [
            "    Build · GPT-6.1 Sol · 692ms",
            "     Build · GPT-6.1 Sol · 2s",
            "     Build · GPT-6.1 Sol · ms",
            "     Build · 692ms",
            "     Build · a · b · 692ms",
            "     Build ·  · 692ms",
            "     Exit code 0.",
            "     two words · GPT · 1ms",
        ] {
            assert!(!is_footer(no), "{no:?}");
        }
    }
}
