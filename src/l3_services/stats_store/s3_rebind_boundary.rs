//! Frozen rebind boundary: event-time evidence at the cursor belongs only to the donor.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Dir, Life, Memory, MemoryJournals, Read};

#[test]
fn s3_rebind_drops_both_line_and_foreign_index_at_the_donor_cursor() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut donor = cases::Feed::live("live/donor", Some(cases::S), cases::birth(cases::S, 1));
    journals.set(donor.id(), 1, donor.ino, &donor.bytes);
    let initial = cases::work(&mut dir, &mut journals, &donor.spec, &Life::default());
    let publication = cases::plan(disk::publish(&mut dir, &mut journals, &initial).expect("donor"));
    cases::complete(&mut dir, &publication);
    donor.cursor = u64::try_from(donor.bytes.len()).expect("consumed prefix length");
    donor.active = true;
    cases::check(&mut dir, std::slice::from_ref(&donor));

    // S2's event door accepts a certificate exactly at the ordered cursor.
    // Append it to the real journal, then publish its event-time evidence.
    let tail = cases::birth(cases::T, 2);
    let line = tail.strip_suffix(b"\n").expect("fixture newline");
    donor.bytes.extend_from_slice(&tail);
    journals.set(donor.id(), 1, donor.ino, &donor.bytes);
    let event = cases::plan(
        disk::event(&mut dir, Some(cases::S), donor.id(), donor.cursor, line)
            .expect("event exactly at cursor"),
    );
    cases::complete(&mut dir, &event);
    donor.early.push((donor.cursor, line.to_vec()));
    cases::check(&mut dir, std::slice::from_ref(&donor));

    let mut recipient = cases::Feed::live("live/recipient", Some(cases::S), donor.bytes.clone());
    recipient.ino = 99;
    journals.set(recipient.id(), 1, recipient.ino, &recipient.bytes);
    let life = Life {
        fingerprint: None,
        donors: vec![donor.id().to_owned()],
    };
    let rebound = cases::work(&mut dir, &mut journals, &recipient.spec, &life);
    assert_eq!(rebound.donor(), Some(donor.id()), "matching donor used");
    assert_eq!(
        rebound.span().1,
        donor.cursor,
        "only consumed prefix copied"
    );
    assert!(
        rebound.more(),
        "recipient must still read the event at its cursor"
    );
    let publication =
        cases::plan(disk::publish(&mut dir, &mut journals, &rebound).expect("rebind"));
    cases::complete(&mut dir, &publication);
    recipient.active = true;
    recipient.cursor = donor.cursor;
    cases::check(&mut dir, &[donor.clone(), recipient.clone()]);

    // T retains the donor's real fragment, but must have no empty fragment
    // from a recipient that has not consumed or received the T certificate.
    let no_prune = disk::prune(&mut dir, Some(cases::T)).expect("T evidence stays readable");
    assert!(
        no_prune.ops.is_empty(),
        "T still has the donor's foreign count"
    );
    let path = format!("stats/session/{}.evidence", cases::T);
    let Read::Bytes(bytes) = dir.read(&path, disk::EVIDENCE_CAP) else {
        panic!("T fragment evidence");
    };
    let text = String::from_utf8(bytes).expect("evidence UTF-8");
    assert!(text.contains(donor.id()));
    assert!(
        !text.contains(recipient.id()),
        "recipient has no T fragment before consuming its tail"
    );

    let continued = cases::work(&mut dir, &mut journals, &recipient.spec, &Life::default());
    assert_eq!(continued.span().0, donor.cursor);
    let publication =
        cases::plan(disk::publish(&mut dir, &mut journals, &continued).expect("tail"));
    cases::complete(&mut dir, &publication);
    recipient.cursor = u64::try_from(recipient.bytes.len()).expect("complete recipient length");
    cases::check(&mut dir, &[donor, recipient]);
}
