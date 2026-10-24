#![no_main]

use ae::archive::harness;
use libfuzzer_sys::fuzz_target;

const AID: &str = "aaaaaaaa-0000-4000-8000-000000000001";

fuzz_target!(|data: &[u8]| {
    if let Ok(record) = harness::parse(data, AID) {
        assert_eq!(harness::encode(&record), data);
        assert_eq!(harness::parse(&harness::encode(&record), AID), Ok(record));
    }
    let (record, _) = harness::from_meta(data, AID);
    let encoded = harness::encode(&record);
    if encoded.len() <= 1 << 20 && record.slots.len() <= 1024 {
        assert_eq!(harness::parse(&encoded, AID), Ok(record));
    }
});
