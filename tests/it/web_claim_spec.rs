//! S1c nonce byte format, authenticated lifetime and durable claim records.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use ae::web::claim::{self, Claim, Claims, Lookup, Outcome, Word};
use ae::web::nonce::{self, Refused};

const EPOCH: u64 = 1_795_000_000;
const DAY: u64 = 86_400;
const KEY: [u8; 32] = [0; 32];
const RANDOM: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];
// Independent Digest::SHA vector for the domain tag, big-endian epoch and random bytes.
const NONCE: &str = concat!(
    "000000006afd86c000112233445566778899aabbccddeeff",
    "e2b537ea60d4c80dd72cc56df4d3f58dae2a97d43d667ab3f18eb0820dc10db0",
);
const NAME: &str = "0513137815aa793e22ed4d230809955a525d3a790aedb9c083d19c400742d08a";
const PAYLOAD: &str = "6d5fe0b4fc3f2bcac7b285ac9a7a04cab2b39cc6bd10e8d52ab2a3f8c207931e";
const ROUTE: &str = "d0c5322cdb4e2b52902137aaf2460064e1193d1e2415789a52ab82cf1bbfc57d";
const ID: &str = "ae-20261122T200000Z-12345678";
const CAP: usize = 4096;

#[test]
fn nonce_mint_and_claim_name_match_the_independent_byte_vectors() {
    assert_eq!(nonce::mint(&KEY, EPOCH, RANDOM), NONCE);
    assert_eq!(nonce::claim_name(NONCE), NAME);
}

#[test]
fn nonce_checks_its_listener_tag_and_the_inclusive_one_day_window() {
    for now in [EPOCH, EPOCH + DAY - 1, EPOCH + DAY] {
        assert!(matches!(nonce::verify(&KEY, NONCE, now), Ok(EPOCH)));
    }
    for now in [EPOCH - 1, EPOCH + DAY + 1] {
        assert!(matches!(
            nonce::verify(&KEY, NONCE, now),
            Err(Refused::Expired)
        ));
    }
    assert!(matches!(
        nonce::verify(&[1; 32], NONCE, EPOCH),
        Err(Refused::Forged)
    ));
    for byte in [7, 8, 55] {
        let mut changed = NONCE.to_owned();
        let position = 2 * byte;
        let before = &NONCE[position..position + 1];
        changed.replace_range(
            position..position + 1,
            if before == "0" { "1" } else { "0" },
        );
        assert!(
            matches!(nonce::verify(&KEY, &changed, EPOCH), Err(Refused::Forged)),
            "tamper byte {byte}"
        );
    }
    for malformed in [
        NONCE.to_uppercase(),
        NONCE[..111].to_owned(),
        format!("{NONCE} "),
    ] {
        assert!(matches!(
            nonce::verify(&KEY, &malformed, EPOCH),
            Err(Refused::Malformed)
        ));
    }
}

fn record() -> String {
    format!("ae-web-claim 1\npayload {PAYLOAD}\nroute {ROUTE}\nid {ID}\ncreated {EPOCH}\n")
}

fn expect_initial(claim: &Claim) {
    assert_eq!(claim.payload, PAYLOAD);
    assert_eq!(claim.route, ROUTE);
    assert_eq!(claim.id, ID);
    assert_eq!(claim.created, EPOCH);
    assert!(claim.outcome.is_none());
}

#[test]
fn the_claim_decoder_rejects_ambiguous_truncated_and_oversized_records() {
    let original = record();
    let initial = claim::decode(original.as_bytes()).unwrap_or_else(|_| panic!("initial claim"));
    expect_initial(&initial);
    assert!(claim::decode(claim::encode(&initial).as_bytes()).is_ok());
    for malformed in [
        original.replace(&format!("route {ROUTE}\n"), ""),
        original.replace("ae-web-claim 1", "ae-web-claim 2"),
        original.replace(PAYLOAD, "wrong-digest"),
        original.replace(ID, "ae-20261122T200000Z-fake0000"),
        original.replace(&EPOCH.to_string(), "-1"),
        format!("{original}id {ID}\n"),
        format!("{original}unexpected field\n"),
        format!("{original}ae-web-claim 1\n"),
        format!("{original}outcome sent\nseat tui\n"),
    ] {
        assert!(
            claim::decode(malformed.as_bytes()).is_err(),
            "{malformed:?}"
        );
    }
    assert!(claim::decode(b"\xff").is_err());

    let prefix = format!("{original}outcome refused\nseat tui\nto sample\nwords ");
    let exact = format!("{prefix}{}\n", "x".repeat(CAP - prefix.len() - 1));
    let too_big = format!("{prefix}{}\n", "x".repeat(CAP - prefix.len()));
    assert_eq!(exact.len(), CAP);
    let refusal = claim::decode(exact.as_bytes()).unwrap_or_else(|_| panic!("4 KiB claim"));
    assert!(matches!(refusal.outcome, Some(outcome) if matches!(outcome.word, Word::Refused)));
    assert_eq!(too_big.len(), CAP + 1);
    assert!(claim::decode(too_big.as_bytes()).is_err());
}

#[allow(
    clippy::disallowed_methods,
    reason = "inspect permissions on the test's own claim"
)]
fn claim_mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("mode {}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn the_claim_store_is_create_new_preserves_the_receipt_and_prunes_only_after_seven_days() {
    let scratch = crate::cli::OwnedScratch::root("wclm", "store");
    let claims_dir = scratch.path().join("web/claims");
    let path = claims_dir.join(NAME);
    let claims = Claims::open(scratch.path());
    assert!(matches!(claims.lookup(NAME), Lookup::Absent));
    let mut initial =
        claim::decode(record().as_bytes()).unwrap_or_else(|_| panic!("initial fixture"));
    claims
        .create(NAME, &initial)
        .unwrap_or_else(|error| panic!("create: {error}"));
    assert_eq!(claim_mode(&path), 0o600);
    match claims.lookup(NAME) {
        Lookup::Known(read) => expect_initial(&read),
        _ => panic!("created claim must be readable before carry"),
    }
    assert!(
        claims.create(NAME, &initial).is_err(),
        "a second claim would admit a second carry"
    );

    initial.outcome = Some(Outcome {
        word: Word::Sent,
        seat: "tui".to_owned(),
        to: "sample".to_owned(),
        words: String::new(),
    });
    claims
        .settle(NAME, &initial)
        .unwrap_or_else(|error| panic!("settle: {error}"));
    assert_eq!(claim_mode(&path), 0o600);
    let open_again = Claims::open(scratch.path());
    let Lookup::Known(settled) = open_again.lookup(NAME) else {
        panic!("claim must survive reopening the store");
    };
    assert_eq!(settled.id, ID);
    assert_eq!(settled.created, EPOCH);
    let result = settled.outcome.unwrap_or_else(|| panic!("stored outcome"));
    assert!(matches!(result.word, Word::Sent));
    assert_eq!(result.seat, "tui");
    assert_eq!(result.to, "sample");

    let damage = "0".repeat(64);
    std::fs::write(claims_dir.join(&damage), "partial record")
        .unwrap_or_else(|error| panic!("damage test node: {error}"));
    assert!(matches!(open_again.lookup(&damage), Lookup::Unreadable));
    open_again.prune(EPOCH + 7 * DAY);
    assert!(matches!(open_again.lookup(NAME), Lookup::Known(_)));
    open_again.prune(EPOCH + 7 * DAY + 1);
    assert!(matches!(open_again.lookup(NAME), Lookup::Absent));
    assert!(matches!(open_again.lookup(&damage), Lookup::Unreadable));
}
