//! R15 + §5 P1 pins for the sanitize slice.
//!
//! Unit tests beside the code pin each function; these pin the COMPOSITION:
//! the INPUT/RENDERED separation, the marker grammar over hostile bytes, and
//! the grep pin on the new module's rendering sites.

#![allow(
    clippy::disallowed_methods,
    reason = "this reads the crate's own source for the grep pin; the capability boundary is \
              about what PRODUCT code may reach"
)]

use std::path::Path;

use ae::event_text::display_cell;
use ae::sanitize::{
    Budgeted, Field, LedgerRef, NameStatus, check_name, invalid_marker, name_marker, omit_marker,
    over_cap_marker, prepare_body, sanitize, select_ids, utf8_skip_reason,
};

/// The product half of the new module — everything above the unit tests,
/// code lines only (comments and doctests may DISCUSS the banned tokens;
/// only code may not USE them). The grep pin scans this, never the tests.
fn product_source() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sanitize.rs");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
    let (product, _) = text
        .split_once("#[cfg(test)]")
        .unwrap_or_else(|| panic!("{} must carry unit tests", path.display()));
    assert!(
        product.contains("pub fn sanitize"),
        "the guard must see the module it guards"
    );
    product
        .lines()
        .filter(|line| !line.trim_start().starts_with('/'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_pasted_body_is_never_passed_through_display_cell() {
    // §5 P1 pin: multi-line + non-ASCII + tab arrives multi-line. display_cell
    // would fold every non-ASCII char and newline to `?` — the sanitize
    // output must differ from that projection exactly there.
    let bytes = "line one\nline two — with é\tand 中\nline three".as_bytes();
    let (clean, n) = sanitize(bytes, Field::Decision).unwrap();
    assert_eq!(n, 0);
    assert_eq!(clean.lines().count(), 3);
    assert!(clean.contains('—') && clean.contains('é') && clean.contains('中'));
    assert_ne!(clean, display_cell(&clean, 4096));
}

#[test]
fn the_rendered_skip_reason_is_already_fully_projected() {
    // The one RENDERED string this slice owns: constants only, so projecting
    // it again is a fixed point, one line, printable ASCII.
    for field in [Field::Goal, Field::Decision] {
        let reason = utf8_skip_reason(field);
        assert_eq!(reason, display_cell(&reason, 1024));
        assert!(!reason.contains('\n'));
        assert!(reason.bytes().all(|b| b.is_ascii_graphic() || b == b' '));
    }
}

#[test]
fn every_marker_is_a_display_cell_fixed_point() {
    // Markers carry constants and counts only — hostile bytes cannot reach
    // them, so projection is always a no-op. If any marker ever interpolates
    // record bytes, this and the grep pin below go red together.
    let markers = [
        omit_marker(Field::Goal, 1),
        omit_marker(Field::Decision, 999_999),
        invalid_marker(3),
        over_cap_marker(17),
        name_marker(),
    ];
    for marker in markers {
        assert_eq!(marker, display_cell(&marker, 1024), "{marker:?}");
        assert!(!marker.contains('\n'), "{marker:?}");
    }
}

#[test]
fn riding_ids_are_display_cell_fixed_points() {
    // Valid ids are clean bounded ASCII by grammar; the report's "projected
    // regardless" is belt-and-braces on top of the check.
    let refs = [
        LedgerRef {
            position: 0,
            bytes: b"ae-20260914T120000Z-00000001",
        },
        LedgerRef {
            position: 1,
            bytes: b"review-20260914T120001Z-abcdef01",
        },
    ];
    let sel = select_ids(&refs);
    assert_eq!(sel.riding.len(), 2);
    for id in &sel.riding {
        assert_eq!(*id, display_cell(id, 64));
    }
}

#[test]
fn no_rendered_site_prints_a_record_field_unprojected() {
    // §5 P1 grep pin, on the new module's rendering sites. Rendering goes
    // through single-line literal-first format! ONLY — every alternate path
    // (print/eprint/println/eprintln/write/writeln, format_args!) is banned
    // outright, because an allowlist over format! arguments cannot see bytes
    // leaving by another macro. Every interpolated argument is a constant or
    // a count — never record bytes. A new site or interpolation outside the
    // allowlist is a review, not a diff (same protocol as doors.rs).
    let product = product_source();
    for forbidden in [
        "write!",
        "writeln!",
        "print!",
        "println!",
        "eprint!",
        "eprintln!",
        "format_args!",
        "display_cell",
        "from_utf8_lossy",
    ] {
        assert!(
            !product.contains(forbidden),
            "src/sanitize.rs product half must not contain {forbidden}"
        );
    }
    let mut formats = 0;
    for line in product.lines() {
        let Some(rest) = line.split_once("format!") else {
            continue;
        };
        formats += 1;
        // Single-line literal-first: the invocation opens on a string
        // literal and closes on this same line. A multiline format! (or a
        // runtime template) evades the argument scan below, so the form
        // itself is pinned, not just the arguments.
        let trimmed = line.trim_end();
        assert!(
            line.contains("format!(\"") && (trimmed.ends_with(')') || trimmed.ends_with(");")),
            "format! must be single-line literal-first: {line:?}"
        );
        // Idents inside braces (inline `{n}` args) plus idents after the
        // literal (positional `{}` args) — both must be allowlisted.
        let (literal, args) = match rest.1.split_once("\", ") {
            Some((literal, args)) => (literal, args),
            None => (rest.1, ""),
        };
        let mut tokens: Vec<&str> = literal
            .split(['{', '}'])
            .skip(1)
            .step_by(2)
            .filter(|t| !t.is_empty())
            .collect();
        tokens.extend(
            args.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .filter(|t| !t.is_empty()),
        );
        for token in tokens {
            assert!(
                ["field", "name", "over", "n"].contains(&token),
                "format! interpolates {token:?} outside the constants-and-counts allowlist: {line:?}"
            );
        }
    }
    assert!(formats > 0, "the guard must see format! calls");
}

#[test]
fn over_budget_fields_are_omitted_whole_with_everything_else_intact() {
    // §9 R15 pin at the pipeline: an 8193-char decision omits whole, the
    // mandatory-composed fields around it are untouched, nothing is cut.
    let decision = "d".repeat(Field::Decision.budget() + 1);
    let body = prepare_body(b"goal", decision.as_bytes(), "worker", &[]).unwrap();
    assert!(matches!(body.goal, Budgeted::Rides(_)));
    assert_eq!(body.ids.invalid, 0);
    assert_eq!(body.name, NameStatus::Rides(String::from("worker")));
    let Budgeted::Omitted { over } = body.decision else {
        panic!("the over-budget decision must omit whole");
    };
    assert_eq!(over, 1);
    assert_eq!(
        omit_marker(Field::Decision, over),
        "(decision omitted: 1 chars over budget)"
    );
}

#[test]
fn a_gated_out_name_omits_whole_and_still_prepares() {
    // Ruling pin, at the property measured: `prepare_body` returns Ok with
    // the name omitted and every other field intact, offending bytes in no
    // record (the marker carries none). Actual dispatch is verb-slice work —
    // this test proves non-refusal, not a paste.
    let body = prepare_body(b"goal", b"decision", "no good", &[]).unwrap();
    assert_eq!(body.name, NameStatus::Omitted);
    assert_eq!(name_marker(), "(name omitted: invalid)");
    assert!(matches!(body.goal, Budgeted::Rides(_)));
    assert!(matches!(body.decision, Budgeted::Rides(_)));
}

#[test]
fn check_name_rejects_control_bytes() {
    assert_eq!(check_name("a\x1bb"), NameStatus::Omitted);
    assert_eq!(
        check_name("worker-1_x"),
        NameStatus::Rides(String::from("worker-1_x"))
    );
}
