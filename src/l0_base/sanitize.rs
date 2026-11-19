//! R15 sanitize + measured budgets for the seat-compact verb (`ae compact`).
//!
//! LOCAL strip, owned by this verb. Record-read bytes (goal line, latest
//! `decision` memo, seat display names, request ids) become terminal INPUT at
//! a seat, so the verb cleans them itself BEFORE the deliver path; `deliver.rs`
//! keeps its verbatim contract and this commit touches it not at all.
//!
//! This strip answers ONE question — "what may this verb paste" — and must not
//! be read as answering whether `deliver.rs` should strip for ALL callers.
//! That is product-wide, its own ruling and slice, UNSETTLED (§8).
//!
//! Surface classes (§5 P1): the pasted body is INPUT (verbatim after this
//! strip, NEVER through `display_cell` — a menu-cell projection would corrupt
//! the load-bearing checkpoint); the skip reason is RENDERED (constants only,
//! record bytes never printed); markers carry constants and counts only.
//!
//! Provenance: R15 + §5 P1 of the frozen compactseats design (lead-frozen
//! 2026-09-14) with lead gap-rulings of the same day — a grammar-failing name
//! is omitted whole with a marker, and record-level UTF-8 fires before id
//! validation. The session files are provenance, not the owner: the contract
//! a fresh checkout can inspect is these docs plus the tests beside them.
//!
//! CONVENTION this file keeps so the `tests/it/sanitize.rs` grep pin stays
//! sound: rendering goes through single-line literal-first `format!` ONLY —
//! no `print!`/`eprint!`/`println!`/`eprintln!`/`write!`/`writeln!`/
//! `format_args!` path exists, because an argument allowlist cannot see bytes
//! leaving by another macro — and interpolated arguments are constants and
//! counts only, never record bytes.

use crate::tracked;

/// An untrusted optional body field read from a persisted record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The session goal line (`meta goal=`).
    Goal,
    /// The latest `decision` checkpoint (`memo.tsv`).
    Decision,
}

impl Field {
    /// The constant name a refusal or marker prints. Record bytes are never
    /// printed — the field word is the whole of what a human sees.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Decision => "decision",
        }
    }

    /// The per-field budget in chars, measured AFTER the strip. Measured, not
    /// guessed (R15 BUDGET); re-measure before changing.
    #[must_use]
    pub const fn budget(self) -> usize {
        match self {
            Self::Goal => 4096,
            Self::Decision => 8192,
        }
    }
}

/// At most this many open request ids ride; the rest are counted, never cut.
pub const MAX_REQUEST_IDS: usize = 16;

/// One ledger ref with its ledger position. Higher `position` is newer; ties
/// keep input order (stable). Positions come from the ledger read; this
/// function assumes nothing about input order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerRef<'a> {
    /// Ledger position; higher is newer.
    pub position: usize,
    /// The raw ref bytes — hostile until checked.
    pub bytes: &'a [u8],
}

/// Sanitize one record field: (1) strict UTF-8, never lossy — invalid refuses
/// the seat and the error carries the field ONLY, never the bytes; (2) line
/// endings to LF (CRLF to one LF, lone CR to LF, U+0085 NEL to LF — CR and
/// NEL are LINE CONTENT in captured terminal output, deleting them joins
/// lines); (3) drop Unicode Cc except LF and TAB, by DECODED CODEPOINT, never
/// by byte range (0x80–0x9F are also UTF-8 continuation bytes).
///
/// No truncation, no ASCII folding, no middle cut. Returns the clean text plus
/// `sanitized` — step-(3) removals ONLY. Step (2) is not counted: a delivered
/// body may differ from the stored record by line-ending normalization without
/// appearing in `sanitized`.
///
/// # Errors
/// Returns the field when its record bytes are not UTF-8 — the refusal that
/// becomes `skipped (record not utf-8: <field>)`. The error carries the field
/// ONLY, never the bytes.
///
/// ```
/// use ae::sanitize::{Field, sanitize};
///
/// let (clean, n) = sanitize(b"a\r\nb\x00", Field::Goal).unwrap();
/// assert_eq!(clean, "a\nb");
/// assert_eq!(n, 1);
/// assert_eq!(sanitize(b"\xff", Field::Goal), Err(Field::Goal));
/// ```
pub fn sanitize(bytes: &[u8], field: Field) -> Result<(String, usize), Field> {
    let text = std::str::from_utf8(bytes).map_err(|_| field)?;
    let mut clean = String::with_capacity(text.len());
    let mut removed = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            clean.push('\n');
        } else if c == '\u{85}' {
            clean.push('\n');
        } else if c == '\n' || c == '\t' || !c.is_control() {
            clean.push(c);
        } else {
            removed += 1;
        }
    }
    Ok((clean, removed))
}

/// The RENDERED skip reason for a step-(1) failure. The field word is a
/// constant; the record bytes are never printed — the PROHIBITION, at the
/// type level: this function takes no bytes.
#[must_use]
pub fn utf8_skip_reason(field: Field) -> String {
    format!("skipped (record not utf-8: {})", field.name())
}

/// A budgeted field: rides whole, or is omitted whole with its overage.
/// Omission is NOT truncation — what is delivered is byte-exact, and the
/// absence is said by [`omit_marker`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Budgeted {
    /// The clean text rides whole.
    Rides(String),
    /// Omitted whole; `over` chars above [`Field::budget`].
    Omitted {
        /// Chars above budget.
        over: usize,
    },
}

/// Budget one sanitized field. Length is CHARS, measured after the strip.
#[must_use]
pub fn apply_budget(clean: &str, field: Field) -> Budgeted {
    let len = clean.chars().count();
    if len <= field.budget() {
        Budgeted::Rides(clean.to_owned())
    } else {
        Budgeted::Omitted {
            over: len - field.budget(),
        }
    }
}

/// The marker an omitted field is replaced by.
/// `(<field> omitted: <n> chars over budget)`.
#[must_use]
pub fn omit_marker(field: Field, over: usize) -> String {
    format!("({} omitted: {} chars over budget)", field.name(), over)
}

/// The riding set plus the two omission counts. The `invalid` and `over_cap`
/// markers are SEPARATE: a valid id over the cap is not "invalid", and both
/// markers may appear together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdSelection {
    /// Valid ids, newest-first by ledger position, at most
    /// [`MAX_REQUEST_IDS`].
    pub riding: Vec<String>,
    /// Refs that failed UTF-8 or [`tracked::is_request_id`], omitted whole.
    pub invalid: usize,
    /// Valid ids past the cap, omitted whole.
    pub over_cap: usize,
}

/// Select the riding set from ledger refs. Each ref is checked against the
/// mint grammar ([`tracked::is_request_id`]); a ref that is not UTF-8 cannot
/// match and counts `invalid`. Valid ids order newest-first by ledger
/// position; the first [`MAX_REQUEST_IDS`] ride. The freshly minted
/// mandatory ref is ae's own and never enters here — exempt by construction,
/// because the caller passes ledger refs only.
///
/// ```
/// use ae::sanitize::{LedgerRef, select_ids};
///
/// let refs = [LedgerRef { position: 0, bytes: b"bogus" }];
/// let sel = select_ids(&refs);
/// assert!(sel.riding.is_empty());
/// assert_eq!(sel.invalid, 1);
/// ```
#[must_use]
pub fn select_ids(refs: &[LedgerRef<'_>]) -> IdSelection {
    let mut valid: Vec<(usize, String)> = Vec::new();
    let mut invalid = 0;
    for r in refs {
        match std::str::from_utf8(r.bytes) {
            Ok(text) if tracked::is_request_id(text) => valid.push((r.position, text.to_owned())),
            _ => invalid += 1,
        }
    }
    valid.sort_by_key(|id| std::cmp::Reverse(id.0));
    let over_cap = valid.len().saturating_sub(MAX_REQUEST_IDS);
    valid.truncate(MAX_REQUEST_IDS);
    IdSelection {
        riding: valid.into_iter().map(|(_, id)| id).collect(),
        invalid,
        over_cap,
    }
}

/// `(<n> request ids omitted: invalid)`.
#[must_use]
pub fn invalid_marker(n: usize) -> String {
    format!("({n} request ids omitted: invalid)")
}

/// `(<n> request ids omitted: over cap)`.
#[must_use]
pub fn over_cap_marker(n: usize) -> String {
    format!("({n} request ids omitted: over cap)")
}

/// A seat display name: rides verbatim when it passes the roster grammar,
/// omitted whole with a marker otherwise. Names are CHECKED, never stripped:
/// the grammar alphabet is clean ASCII by construction, and a name that
/// fails an allowlist it passed at creation means tampered persisted state —
/// recorded, not quietly cleaned.
///
/// Non-UTF-8 is unrepresentable here BY TYPE (`&str`): names arrive as
/// `RosterEntry.name: String` through `Meta::read`'s strict
/// `fs::read_to_string`, which fails the whole read before any name exists
/// (ruling file). No `not utf-8` name marker exists, on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameStatus {
    /// The name rides verbatim.
    Rides(String),
    /// Omitted whole; the verb composes the ruled marker.
    Omitted,
}

/// Gate one seat display name against the roster grammar
/// ([`crate::config::is_agent_name`], the one owner).
#[must_use]
pub fn check_name(name: &str) -> NameStatus {
    if crate::config::is_agent_name(name) {
        NameStatus::Rides(name.to_owned())
    } else {
        NameStatus::Omitted
    }
}

/// `(name omitted: invalid)` — the ruled marker for a gated-out name, in
/// R15's `(<field> omitted: <reason>)` shape. Lead-ruled 2026-09-14: the
/// one-word field parallels `goal`/`decision`, the reason reuses R15's
/// `invalid` (a value that cannot match its grammar), and the same word binds
/// the verb slice's body spelling. The exact bytes ship from here — this
/// function is their tracked owner of record.
///
/// There is deliberately NO `not utf-8` name marker: non-UTF-8 is
/// unrepresentable at this site by type (`&str` — names arrive as
/// `RosterEntry.name: String` through `Meta::read`'s strict whole-file
/// `fs::read_to_string`, which fails before any name exists). If the type
/// ever admits non-UTF-8, `not utf-8` is the RESERVED word — say that here
/// so nobody invents a synonym later.
#[must_use]
pub fn name_marker() -> String {
    String::from("(name omitted: invalid)")
}

/// Everything the verb composes into one seat's checkpoint body, after R15.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedBody {
    /// The budgeted goal line.
    pub goal: Budgeted,
    /// The budgeted latest `decision` checkpoint.
    pub decision: Budgeted,
    /// The gated seat display name.
    pub name: NameStatus,
    /// The selected open request ids involving this seat.
    pub ids: IdSelection,
    /// Total step-(3) removals across goal and decision. Names and ids
    /// contribute none: names are gated, never stripped, and ids match a
    /// clean grammar or are omitted.
    pub sanitized: usize,
}

/// The verb's one R15 entrypoint. ORDER IS THE CONTRACT (lead ruling):
/// record-level UTF-8 on the load-bearing fields fires FIRST — a goal or
/// memo that is not UTF-8 refuses the seat (`Err` carries the field only)
/// and id validation is NEVER reached. Only valid-UTF-8 records flow on to
/// grammar checks and budgets, where failures DEGRADE to markers: the seat
/// is never skipped for size, an invalid id, or a bad name.
///
/// # Errors
/// Returns the field when the goal or decision record bytes are not UTF-8.
/// Id validation is never reached on this path — no selection comes back.
pub fn prepare_body(
    goal: &[u8],
    decision: &[u8],
    name: &str,
    refs: &[LedgerRef<'_>],
) -> Result<PreparedBody, Field> {
    let (goal_text, goal_removed) = sanitize(goal, Field::Goal)?;
    let (decision_text, decision_removed) = sanitize(decision, Field::Decision)?;
    Ok(PreparedBody {
        goal: apply_budget(&goal_text, Field::Goal),
        decision: apply_budget(&decision_text, Field::Decision),
        name: check_name(name),
        ids: select_ids(refs),
        sanitized: goal_removed + decision_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_terminator_and_c0_bytes_are_stripped_and_counted() {
        // R15 pin: the bracketed-paste terminator plus C0 bytes.
        let bytes = b"keep\x1b[201~gone\x00\x07\x1bend";
        let (clean, n) = sanitize(bytes, Field::Goal).unwrap();
        assert_eq!(clean, "keep[201~goneend");
        assert_eq!(n, 4);
    }

    #[test]
    fn c1_is_removed_by_codepoint_and_non_ascii_survives() {
        // U+009B is bytes C2 9B; a byte-range filter on 0x80-0x9F guts the
        // em dash (E2 80 94), the e-acute (C3 A9) and the CJK (E4 B8 AD).
        let bytes = "a—\u{9b}béc中d".as_bytes();
        let (clean, n) = sanitize(bytes, Field::Decision).unwrap();
        assert_eq!(clean, "a—béc中d");
        assert_eq!(n, 1);
    }

    #[test]
    fn nel_becomes_lf_and_is_not_counted() {
        let (clean, n) = sanitize(b"a\xc2\x85b", Field::Goal).unwrap();
        assert_eq!(clean, "a\nb");
        assert_eq!(n, 0);
    }

    #[test]
    fn bare_cr_becomes_lf_and_keeps_the_line_count() {
        // A bare-\r progress-bar capture: deleting CR joins lines; the pin
        // requires the COUNT to survive.
        let (clean, n) = sanitize(b"one\rtwo\rthree", Field::Goal).unwrap();
        assert_eq!(clean, "one\ntwo\nthree");
        assert_eq!(clean.lines().count(), 3);
        assert_eq!(n, 0);
    }

    #[test]
    fn crlf_becomes_one_lf_and_is_not_counted() {
        let (clean, n) = sanitize(b"a\r\nb\r\n", Field::Goal).unwrap();
        assert_eq!(clean, "a\nb\n");
        assert_eq!(n, 0);
    }

    #[test]
    fn tab_lf_and_del() {
        let (clean, n) = sanitize(b"a\tb\nc\x7fd", Field::Goal).unwrap();
        assert_eq!(clean, "a\tb\ncd");
        assert_eq!(n, 1);
    }

    #[test]
    fn invalid_utf8_refuses_with_the_field_only() {
        assert_eq!(sanitize(b"ok\xffbad", Field::Goal), Err(Field::Goal));
        assert_eq!(sanitize(b"\xfe", Field::Decision), Err(Field::Decision));
    }

    #[test]
    fn skip_reason_is_exact_and_carries_no_bytes() {
        assert_eq!(
            utf8_skip_reason(Field::Goal),
            "skipped (record not utf-8: goal)"
        );
        assert_eq!(
            utf8_skip_reason(Field::Decision),
            "skipped (record not utf-8: decision)"
        );
    }

    #[test]
    fn budget_boundary_rides_at_cap_and_omits_whole_above() {
        let at_cap = "d".repeat(Field::Decision.budget());
        assert_eq!(
            apply_budget(&at_cap, Field::Decision),
            Budgeted::Rides(at_cap.clone())
        );
        let over = "d".repeat(Field::Decision.budget() + 1);
        assert_eq!(
            apply_budget(&over, Field::Decision),
            Budgeted::Omitted { over: 1 }
        );
        let goal_cap = "g".repeat(Field::Goal.budget());
        assert!(matches!(
            apply_budget(&goal_cap, Field::Goal),
            Budgeted::Rides(_)
        ));
        let goal_over = "g".repeat(Field::Goal.budget() + 1);
        assert_eq!(
            apply_budget(&goal_over, Field::Goal),
            Budgeted::Omitted { over: 1 }
        );
    }

    #[test]
    fn budget_counts_chars_not_bytes() {
        // 4096 em dashes are 12288 bytes but 4096 chars: rides.
        let dashes: String = "\u{2014}".repeat(Field::Goal.budget());
        assert!(matches!(
            apply_budget(&dashes, Field::Goal),
            Budgeted::Rides(_)
        ));
        let one_more = format!("{dashes}\u{2014}");
        assert_eq!(
            apply_budget(&one_more, Field::Goal),
            Budgeted::Omitted { over: 1 }
        );
    }

    #[test]
    fn omit_marker_is_exact() {
        assert_eq!(
            omit_marker(Field::Decision, 1),
            "(decision omitted: 1 chars over budget)"
        );
        assert_eq!(
            omit_marker(Field::Goal, 40),
            "(goal omitted: 40 chars over budget)"
        );
    }

    fn ledger<'a>(refs: &'a [&'a [u8]]) -> Vec<LedgerRef<'a>> {
        refs.iter()
            .enumerate()
            .map(|(i, bytes)| LedgerRef { position: i, bytes })
            .collect()
    }

    #[test]
    fn select_ids_rides_newest_first_and_caps_at_sixteen() {
        let mut refs: Vec<Vec<u8>> = Vec::new();
        for i in 0..17 {
            refs.push(format!("ae-20260914T1200{i:02}Z-000000{i:02}").into_bytes());
        }
        // 17 valid ids, position 16 newest: 16 ride newest-first, 1 over cap.
        let borrowed: Vec<&[u8]> = refs.iter().map(Vec::as_slice).collect();
        let sel = select_ids(&ledger(&borrowed));
        assert_eq!(sel.riding.len(), 16);
        assert_eq!(sel.riding[0], "ae-20260914T120016Z-00000016");
        assert_eq!(sel.riding[15], "ae-20260914T120001Z-00000001");
        assert_eq!(sel.invalid, 0);
        assert_eq!(sel.over_cap, 1);
    }

    #[test]
    fn select_ids_invalid_and_over_cap_markers_cooccur() {
        let mut refs: Vec<Vec<u8>> = Vec::new();
        for i in 0..17 {
            refs.push(format!("ae-20260914T1200{i:02}Z-000000{i:02}").into_bytes());
        }
        refs.push(vec![b'x'; 10_000]);
        let borrowed: Vec<&[u8]> = refs.iter().map(Vec::as_slice).collect();
        let sel = select_ids(&ledger(&borrowed));
        assert_eq!(sel.riding.len(), 16);
        assert_eq!(sel.invalid, 1);
        assert_eq!(sel.over_cap, 1);
        assert_eq!(
            invalid_marker(sel.invalid),
            "(1 request ids omitted: invalid)"
        );
        assert_eq!(
            over_cap_marker(sel.over_cap),
            "(1 request ids omitted: over cap)"
        );
    }

    #[test]
    fn select_ids_counts_non_utf8_as_invalid() {
        let refs = [&b"ae-20260914T120000Z-00000001"[..], &b"\xff\xfe"[..]];
        let sel = select_ids(&ledger(&refs));
        assert_eq!(sel.riding, ["ae-20260914T120000Z-00000001"]);
        assert_eq!(sel.invalid, 1);
        assert_eq!(sel.over_cap, 0);
    }

    #[test]
    fn check_name_gates_grammar() {
        assert_eq!(
            check_name("worker"),
            NameStatus::Rides(String::from("worker"))
        );
        assert_eq!(check_name("no good"), NameStatus::Omitted);
        assert_eq!(check_name(""), NameStatus::Omitted);
        assert_eq!(check_name("a\x1bb"), NameStatus::Omitted);
    }

    #[test]
    fn name_marker_is_exact() {
        assert_eq!(name_marker(), "(name omitted: invalid)");
    }

    #[test]
    fn prepare_body_fires_utf8_before_id_validation() {
        // Ordering 1: a non-UTF-8 goal refuses the seat no matter the refs —
        // id validation is never reached, so no selection comes back.
        let refs = [&b"ae-20260914T120000Z-00000001"[..]];
        assert_eq!(
            prepare_body(b"\xff", b"decision", "worker", &ledger(&refs)),
            Err(Field::Goal)
        );
        assert_eq!(
            prepare_body(b"goal", b"\xfe", "worker", &ledger(&refs)),
            Err(Field::Decision)
        );
    }

    #[test]
    fn prepare_body_degrades_ids_and_names_to_markers() {
        // Ordering 2: valid UTF-8 records flow on; bad refs and a bad name
        // degrade — the seat is never skipped for them.
        let refs = [&b"bogus"[..], &b"ae-20260914T120000Z-00000001"[..]];
        let body = prepare_body(b"goal", b"decision", "no good", &ledger(&refs)).unwrap();
        assert!(matches!(body.goal, Budgeted::Rides(_)));
        assert!(matches!(body.decision, Budgeted::Rides(_)));
        assert_eq!(body.name, NameStatus::Omitted);
        assert_eq!(body.ids.invalid, 1);
        assert_eq!(body.ids.riding.len(), 1);
        assert_eq!(body.sanitized, 0);
    }

    #[test]
    fn prepare_body_sums_sanitized_over_goal_and_decision_only() {
        let refs = [&b"ae-20260914T120000Z-00000001"[..]];
        let body = prepare_body(b"g\x00", b"d\x07\x1b", "worker", &ledger(&refs)).unwrap();
        assert_eq!(body.sanitized, 3);
    }
}
