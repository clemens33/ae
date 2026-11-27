//! The launch context a seat reads, frozen as bytes before the leadership block
//! is extracted (apppolish 2a). Oracle: goldens recorded from base 4aad1af0 by
//! `ae _context <dir> s /w <slot>`; the session dir is normalised to `<DIR>`.
//! The pair is the two corners of quota-aware x peer that a swapped argument
//! cannot survive: solo + aware, lead-pair + unaware. The worker card is the
//! third. The remaining corners are pinned by the render owner's own tests.
//! Relay config acceptance preserves that byte oracle except for the two names
//! in clause (3), which the generic guard replaces.

use ae::render::context_document;

const SOLO: &str = "mode=local\nseat.main=lead\n";
const PAIR: &str = "mode=local\nlayout=lead-pair\nschema=2\nseat.main=lead\nseat.worker.0=colead\n";

/// The document for `slot` of a session whose meta is `meta`, with the
/// scratch dir spelled `<DIR>` and `quota = off` when `unaware`.
fn rendered(tag: &str, meta: &str, slot: &str, unaware: bool) -> String {
    let scratch = super::cli::OwnedScratch::root("lctx", tag);
    let dir = scratch.path();
    assert!(std::fs::write(dir.join("meta"), meta).is_ok(), "meta");
    let config = dir.join("off");
    assert!(
        std::fs::write(&config, "[workspace]\nquota = off\n").is_ok(),
        "config"
    );
    let files = if unaware { vec![config] } else { Vec::new() };
    context_document(dir, "s", "/w", slot, &files).replace(&dir.display().to_string(), "<DIR>")
}

#[test]
fn a_solo_main_seat_reads_the_aware_leadership_context_byte_for_byte() {
    assert_eq!(
        rendered("solo", SOLO, "main", false),
        include_str!("../fixtures/launch-context-solo-main-aware.golden")
    );
}

#[test]
fn a_lead_pair_main_seat_reads_the_unaware_peer_context_byte_for_byte() {
    assert_eq!(
        rendered("pair", PAIR, "main", true),
        include_str!("../fixtures/launch-context-pair-main-unaware.golden")
    );
}

#[test]
fn a_spawned_worker_reads_the_worker_card_context_byte_for_byte() {
    let meta = format!("{SOLO}seat.spawned.4=builder\n");
    assert_eq!(
        rendered("worker", &meta, "spawned.4", false),
        include_str!("../fixtures/launch-context-spawned-worker.golden")
    );
}

// #247 policy ruling: expected text is frozen independently of the renderer.
#[test]
fn pacewords_forecast_steers_new_work_without_banning_a_scope() {
    const PACE: &str = r"Read PACE beside EFFECTIVE: `!` or an ETA before RESETS is a forecast, never a stop. It steers NEW work toward lower effort, more EFFECTIVE headroom and sensible concurrency, task quality kept. A short window near its reset returns soon; weekly headroom is the scarce one: at 80% EFFECTIVE weekly step effort down, at 95% park the scope (no NEW work). Only exhausted, blocked, spend-cap or a vendor usage-limit line removes a scope now; parking one never stops the others. Report a forecast with its adjustment and uncertainty. `unknown` or `-` neither blocks nor proves headroom; PACE never replaces EFFECTIVE.";
    const LOW: &str = r"At low or critical WEEKLY headroom, or when a seat's spend (`<DIR>/usage`) is out of proportion to its task class, step DOWN before switching provider: effort xhigh → high for bounded work, a cheaper chore-tier profile for chores; xhigh stays for judgment seats only.";
    let document = rendered("pacewords", SOLO, "main", false);
    assert!(document.contains(PACE), "{document}");
    assert!(document.contains(LOW), "{document}");
    assert!(
        !document.contains("choose another profile now"),
        "{document}"
    );
    assert!(
        !document.contains("At low or critical headroom"),
        "{document}"
    );
    let unaware = rendered("pacewords-off", SOLO, "main", true);
    assert!(
        !unaware.contains(PACE),
        "quota off keeps the guidance absent"
    );
}

const RESERVED_SYSTEM: &str =
    "a system the human named as reserved, or of any production system when none is named";
const RESERVED_ORGANISATION: &str = "the human's employer";

fn reserved_config(systems: &str, organisation: &str) -> String {
    format!("[workspace]\nreserved_systems = {systems}\nreserved_organisation = {organisation}\n")
}

#[allow(clippy::expect_used, reason = "private context-rendering fixture")]
fn relay_context(tag: &str, global: Option<&str>, local: Option<&str>) -> String {
    let scratch = super::cli::OwnedScratch::root("lctx", tag);
    let dir = scratch.path();
    let mut meta = SOLO.to_owned();
    let mut command = super::cli::ae();
    command.arg("_context").arg(dir).args(["s", "/w", "main"]);
    for (flag, text) in [("--global", global), ("--local", local)] {
        if let Some(text) = text {
            let file = dir.join(&flag[2..]);
            std::fs::write(&file, text).expect("fixture config");
            command.arg(flag).arg(&file);
            if flag == "--global" {
                use std::fmt::Write as _;
                writeln!(meta, "config={}", file.display()).expect("write meta row");
            }
        }
    }
    std::fs::write(dir.join("meta"), meta).expect("session meta");
    let output = command.output().expect("context renderer runs");
    assert!(
        output.status.success(),
        "{tag}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("context is UTF-8")
        .replace(&dir.display().to_string(), "<DIR>")
}

fn reserved_context_expected(systems: &str, organisation: &str) -> String {
    include_str!("../fixtures/launch-context-solo-main-aware.golden")
        .replace(RESERVED_SYSTEM, systems)
        .replace(RESERVED_ORGANISATION, organisation)
}

#[test]
fn reserved_relay_uses_only_the_global_setting_and_preserves_every_other_byte() {
    let alternative = reserved_config("Warehouse", "Another Company");
    let configured = reserved_config("Payments, Billing", "Example Corp");
    let expected = reserved_context_expected("Payments or Billing", "Example Corp");
    for (tag, local) in [("names", None), ("overlay", Some(alternative.as_str()))] {
        let context = relay_context(tag, Some(&configured), local);
        assert!(
            context == expected,
            "{tag}: reserved launch context differs"
        );
    }

    let fallback = reserved_context_expected(RESERVED_SYSTEM, RESERVED_ORGANISATION);
    for (tag, local) in [("unset", None), ("only-local", Some(alternative.as_str()))] {
        let context = relay_context(tag, None, local);
        assert!(context == fallback, "{tag}: generic guard differs");
    }

    for (tag, systems, written) in [
        ("single", "Warehouse", "Warehouse"),
        (
            "three",
            "Payments, Billing, Caisse-Épargne",
            "Payments, Billing or Caisse-Épargne",
        ),
    ] {
        let input = reserved_config(systems, "\"Example Corp\"");
        let context = relay_context(tag, Some(&input), None);
        assert!(
            context == reserved_context_expected(written, "Example Corp"),
            "{tag}: configured system names or organisation differ"
        );
    }
}

#[test]
fn reserved_relay_refuses_only_the_bad_value_and_keeps_each_guard() {
    let overlong = "A".repeat(65);
    for (tag, systems, organisation, expected_system, expected_organisation) in [
        ("empty", "", "Example Corp", RESERVED_SYSTEM, "Example Corp"),
        (
            "gap",
            "Payments, , Billing",
            "Example Corp",
            RESERVED_SYSTEM,
            "Example Corp",
        ),
        (
            "symbol",
            "Payments",
            "Example [Corp]",
            "Payments",
            RESERVED_ORGANISATION,
        ),
        (
            "control",
            "Pay\tments",
            "Example Corp",
            RESERVED_SYSTEM,
            "Example Corp",
        ),
        (
            "long-system",
            &overlong,
            "Example Corp",
            RESERVED_SYSTEM,
            "Example Corp",
        ),
        (
            "long-org",
            "Payments",
            &overlong,
            "Payments",
            RESERVED_ORGANISATION,
        ),
    ] {
        let broken = reserved_config(systems, organisation);
        let context = relay_context(tag, Some(&broken), None);
        assert!(
            context == reserved_context_expected(expected_system, expected_organisation),
            "{tag}: the bad value must fall back and the valid value must stay"
        );
    }

    let mut duplicate = reserved_config("Payments", "Example Corp");
    duplicate.push_str("reserved_systems = Warehouse\n");
    let context = relay_context("duplicate", Some(&duplicate), None);
    assert!(
        context == reserved_context_expected(RESERVED_SYSTEM, "Example Corp"),
        "a duplicate must refuse the key, whichever occurrence came last"
    );
}
