//! The launch context a seat reads, frozen as bytes before the leadership block
//! is extracted (apppolish 2a). Oracle: goldens recorded from base 4aad1af0 by
//! `ae _context <dir> s /w <slot>`; the session dir is normalised to `<DIR>`.
//! The pair is the two corners of quota-aware x peer that a swapped argument
//! cannot survive: solo + aware, lead-pair + unaware. The worker card is the
//! third. The remaining corners are pinned by the render owner's own tests.

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
