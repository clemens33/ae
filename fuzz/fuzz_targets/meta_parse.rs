#![no_main]

use libfuzzer_sys::fuzz_target;

// Session meta: hand-editable persisted state, parsed infallibly into a
// document plus a list of anomalies. Every byte reaches the key/value split.
// The stopped-rename intent is the same hostile class (a crashed write or a
// hand edit plants it): the same target drives its typed validator, so the
// transaction's recovery carrier has parser coverage before cutover.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::meta::Meta::parse(&text));
    let _ = std::hint::black_box(ae::meta::meta_agent_role(data));
    let _ = std::hint::black_box(ae::rename::parse_intent(data));
    // The PREDECESSOR row is a second grammar on top of the meta parse: each
    // element says which tool owns the conversation, and a hand edit or a
    // crashed write reaches that split directly. The tool a writer would tag
    // with comes off the same hostile document, so it is driven from it here.
    let parsed = ae::meta::Meta::parse(&text);
    for slot in ["main", "worker.0", "spawned.1"] {
        let raw = parsed.harness_session_prior(slot);
        for element in &raw {
            let _ = std::hint::black_box(ae::meta::prior_parts(element));
        }
        let _ = std::hint::black_box(ae::meta::priors_tagged(&raw, &text));
    }
    // The seat-dir row is judged from RAW bytes before any parse trusts it:
    // presence, duplication, bareness and UTF-8 are byte-level verdicts the
    // resume enlistment refuses on. Same fixed slots as the roster reads.
    for slot in ["main", "worker.0", "spawned.1"] {
        let _ = std::hint::black_box(ae::meta::raw_seat_work_dir(data, slot));
    }
});
