#![no_main]

use ae::birth::{Family, Record, decode, durable_line, is_family};
use libfuzzer_sys::fuzz_target;

// One journal line through the cutover decoder, `Event::parse_line` then
// `birth::decode`, never a parallel parser. The trim's verdict and the decoder
// agree on what the family is: only a parsed event is durable, every family
// record is classified, and a classified identity is never empty.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let durable = durable_line(&text);
    let Ok(event) = ae::events::Event::parse_line(&text) else {
        assert!(!durable);
        return;
    };
    let family = is_family(&event.action);
    assert_eq!(durable, family);
    match decode(&event) {
        None => assert!(!family),
        Some(Ok(Record::Birth(birth))) => assert!(!birth.birth.is_empty()),
        Some(Ok(Record::Fact(fact))) => assert!(!fact.kind.is_empty() && !fact.birth.is_empty()),
        Some(Ok(Record::SpawnLink { birth })) => {
            assert!(!birth.is_empty() && event.action == "spawn");
        }
        Some(Ok(Record::Unsupported {
            family, version, ..
        })) => {
            assert!(version > ae::birth::VERSION && family != Family::Spawn);
        }
        Some(Err(damage)) => {
            assert!(
                damage
                    .birth
                    .as_deref()
                    .is_none_or(|birth| !birth.is_empty())
            );
        }
    }
});
