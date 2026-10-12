#![no_main]

use ae::{provenance, tracked};
use libfuzzer_sys::fuzz_target;

const ID: &str = "ae-20260930T060000Z-0000abcd";
const REPLY: &str = "/fixture/reply \"<your reply>\"";

// The console's reader of a stored ask body (`tracked::uncompose_ask`) over
// hostile bytes. The first byte sizes the id the stored text is read for, taken
// from the front of the rest; what is left is the stored file, lossily as text.
// Whatever it is, the answer is `None` or a piece of that very text behind the
// exact peer marker, and the human's own words, whatever markers they carry,
// come back byte for byte from the envelope the delivery frames them in.
fuzz_target!(|data: &[u8]| {
    let (size, rest) = data.split_first().map_or((0, data), |(n, rest)| (*n, rest));
    let (id, stored) = rest.split_at(usize::from(size).min(rest.len()));
    let (id, stored) = (String::from_utf8_lossy(id), String::from_utf8_lossy(stored));
    if let Some(body) = tracked::uncompose_ask(&stored, &id) {
        let (from, to) = (
            stored.as_ptr() as usize,
            stored.as_ptr() as usize + stored.len(),
        );
        let at = body.as_ptr() as usize;
        assert!(
            from <= at && at + body.len() <= to,
            "a piece of the stored text"
        );
        assert!(
            stored.starts_with("⟦ae:msg from "),
            "behind the peer marker"
        );
    }
    for sender in [tracked::CONSOLE_DISPLAY, tracked::CONSOLE_SINK] {
        let framed = provenance::first_line(
            &provenance::peer(sender),
            &tracked::compose(tracked::Kind::Ask, ID, sender, &stored, REPLY),
        );
        assert_eq!(tracked::uncompose_ask(&framed, ID), Some(&*stored));
    }
});
