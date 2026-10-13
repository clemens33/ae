#![no_main]

use ae::events::{Cursor, EventLog, Held};
use ae::store::SourceRead;
use libfuzzer_sys::fuzz_target;

// First two bytes size an initial journal; subsequent op/size/payload triples
// append, rewrite in place, shrink, or replace. Every step checks both readers
// against the whole parsers, including strict reads primed by a follow read.
fuzz_target!(|data: &[u8]| {
    let Some(head) = data.get(..2) else {
        return;
    };
    let rest = &data[2..];
    let size = usize::from(u16::from_be_bytes([head[0], head[1]])).min(rest.len());
    let (initial, mut ops) = rest.split_at(size);
    let mut bytes = initial.to_vec();
    let (mut follow, mut strict, mut shared) = (Held::default(), Held::default(), Held::default());
    let mut inode = 1;
    loop {
        let key = Some((0, inode));
        let whole = EventLog::parse_bytes(&bytes, Cursor::default());
        assert_eq!(follow.read_bytes(bytes.clone(), key), &whole);
        assert_eq!(shared.read_bytes(bytes.clone(), key), &whole);
        let expected = ae::console::snapshot(SourceRead::Ready(bytes.clone()));
        assert_eq!(
            ae::console::snapshot_at(&mut strict, SourceRead::Ready(bytes.clone()), key),
            expected
        );
        assert_eq!(
            ae::console::snapshot_at(&mut shared, SourceRead::Ready(bytes.clone()), key),
            expected
        );
        let Some(head) = ops.get(..2) else {
            break;
        };
        let payload_len = usize::from(head[1]).min(ops.len() - 2);
        let (payload, tail) = ops[2..].split_at(payload_len);
        match head[0] % 4 {
            0 => bytes.extend_from_slice(payload),
            1 if !bytes.is_empty() => {
                let length = bytes.len();
                for (at, byte) in payload.iter().enumerate() {
                    bytes[(usize::from(head[1]) + at) % length] = *byte;
                }
            }
            2 => bytes.truncate(usize::from(head[1]) % (bytes.len() + 1)),
            3 => {
                bytes = payload.to_vec();
                inode += 1;
            }
            _ => {}
        }
        ops = tail;
    }
});
