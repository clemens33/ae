//! Frozen credits acceptance: R4 doctrine text (D1-D12 of the critique).
//!
//! INSTALL (driver, byte-identical): copy to
//! `tests/it/credits_doctrine_spec.rs` and add `mod credits_doctrine_spec;`
//! to `tests/it/main.rs` between `mod copilot_spec;` and `mod daemons;`
//! Oracle: `.local/brief-credits.md` R4 and `.local/credits/nav-critique.md`
//! rulings D1-D12. Whitespace-normalized pins, so doc line wraps do not matter.
//! On base the new sentences are absent and the old ones present: RED.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance reads the repo's own reference docs"
)]

use std::path::Path;

use super::cli::OwnedScratch;
use ae::render::context_document;

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn norm(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn check(path: &str, absent: &[&str], present: &[&str]) {
    let text = norm(&read(path));
    for old in absent.iter().copied() {
        assert!(!text.contains(old), "{path} still says: {old}");
    }
    for new in present.iter().copied() {
        assert!(text.contains(new), "{path} lacks: {new}");
    }
}

#[test]
fn guidance_names_the_credit_balance() {
    let scratch = OwnedScratch::root("credits", "guidance");
    std::fs::write(scratch.path().join("meta"), "schema=2\nseat.main=lead\n")
        .expect("scratch meta");
    let document = context_document(scratch.path(), "s", "/w", "main", &[]);
    assert!(
        !document.contains("unlimited credits is not near its limit"),
        "{document}"
    );
    assert!(
        document.contains("a scope with POSITIVE declared manual resets, unlimited credits, or a usable credit balance is not near its limit at a high raw percentage"),
        "{document}"
    );
    assert!(
        document.contains("A scope that runs on credits spends a paid balance: choose it only when no scope with window headroom fits the task and the review rules, and step effort down there."),
        "credits are a fallback pool, never a preference: {document}"
    );
}

#[test]
fn dialog_note_lists_runs_on_credits() {
    check(
        "docs/reference/commands.md",
        &["(`1 reset in hand`, `credits unlimited`, `spend cap reached`)"],
        &[
            "(`1 reset in hand`, `credits unlimited`, `runs on credits`, `spend cap reached`)",
            "A window running on a credit balance keeps its vendor percentage, so `COUNTS` stays absent and `NOTE` carries `runs on credits`.",
        ],
    );
}

#[test]
fn credits_cell_documents_normalization() {
    check(
        "docs/reference/commands.md",
        &[],
        &[
            "A balance literal that is a plain non-negative decimal reads normalized — trailing fractional zeros dropped, then a bare trailing dot (`62500.0000000000` reads `62500`, `12.50` reads `12.5`) — never rounded or clipped; a literal longer than 9 characters after that, or not a plain decimal, reads `available`.",
        ],
    );
}

#[test]
fn relief_rule_names_usable_credits() {
    check(
        "docs/reference/commands.md",
        &[
            "while an unlimited-credit claim — the one fact that ADDS headroom — relieves a window only when it was reported no earlier than that window's own observation.",
        ],
        &[
            "while a usable-credit claim — unlimited credits, a balance above zero, or credits available in an unstated amount; the one fact that ADDS headroom — relieves a window only when it was reported no earlier than that window's own observation.",
        ],
    );
}

#[test]
fn effective_cell_documents_cr_mark() {
    check(
        "docs/reference/commands.md",
        &[],
        &[
            "Unlimited credits read `0%`, because the window does not bind.",
            "A usable credit balance reads the window's own percentage with a `cr` mark (`96% cr`), because the window binds no threshold while credits carry it.",
        ],
    );
}

#[test]
fn pace_absence_covers_credit_balance() {
    check(
        "docs/reference/commands.md",
        &["the account is unlimited or spend-capped, another row"],
        &[
            "the account is unlimited or spend-capped, or the window runs on a credit balance, another row",
        ],
    );
}

#[test]
fn machine_balance_documents_normalization() {
    check(
        "docs/reference/commands.md",
        &["`balance` is the vendor's exact decimal literal as a number token"],
        &[
            "`balance` is the vendor's decimal literal, normalized as the `CREDITS` cell normalizes it, as a number token",
        ],
    );
}

#[test]
fn machine_derivation_lists_credits() {
    check(
        "docs/reference/commands.md",
        &[
            "otherwise `derivation` is `\"xN\"` (N declared resets), `\"unlimited\"` or `\"spend-cap\"`.",
        ],
        &[
            "otherwise `derivation` is `\"xN\"` (N declared resets), `\"unlimited\"`, `\"credits\"` (a usable balance carries the window) or `\"spend-cap\"`.",
        ],
    );
}

#[test]
fn advisory_thresholds_name_the_credit_balance() {
    check(
        "docs/reference/commands.md",
        &["so a window with a declared reset in hand or unlimited credits does not raise one"],
        &[
            "so a window with a declared reset in hand, unlimited credits, or a usable credit balance does not raise one",
        ],
    );
}

#[test]
fn delegation_names_the_credit_balance() {
    check(
        "docs/reference/delegation.md",
        &["a declared manual reset or unlimited credits means the client is not at its limit"],
        &[
            "a declared manual reset, unlimited credits, or a usable credit balance means the client is not at its limit",
            "A scope that runs on credits spends a paid balance: choose it only when no scope with window headroom fits the task and the review rules, and step effort down there.",
        ],
    );
}

#[test]
fn contract_derivation_names_usable_credits() {
    check(
        "docs/reference/contract.md",
        &[
            "a reached spend cap outranks unlimited credits, which outrank",
            "The dialog names the derivation (`xN`, `unlimited` or `spend-cap`)",
        ],
        &[
            "a reached spend cap outranks usable credits (unlimited credits, a balance above zero, or credits available in an unstated amount), which outrank",
            "The dialog names the derivation (`xN`, `unlimited`, `credits` or `spend-cap`)",
        ],
    );
}

#[test]
fn contract_relief_names_usable_credits() {
    check(
        "docs/reference/contract.md",
        &[
            "while an unlimited-credit claim relieves a window only when it was reported no earlier than that window's own observation",
            "A balance ae cannot state exactly is reported as available WITHOUT an amount, never clipped to a different number.",
        ],
        &[
            "while a usable-credit claim (unlimited credits, a balance above zero, or credits available in an unstated amount) relieves a window only when it was reported no earlier than that window's own observation",
            "A balance literal that is a plain non-negative decimal is first normalized exactly (trailing fractional zeros dropped, then a bare trailing dot); a balance ae then cannot state exactly is reported as available WITHOUT an amount, never clipped to a different number.",
        ],
    );
}
