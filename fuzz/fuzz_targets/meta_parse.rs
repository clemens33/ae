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
        let _ = std::hint::black_box(parsed.launch_id(slot));
        let _ = std::hint::black_box(parsed.done_confirmations_pin());
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
    // The seat-dir resolvers and the selection/containment policy run over the
    // PARSED document: the string resolve, the typed resolve against a FIXED
    // synthetic session canonical, and containment of any resolved place
    // against FIXED synthetic roots. Pure entrypoints only — no filesystem
    // call here, and never a fuzz-selected host path.
    let roots = [
        std::path::PathBuf::from("/state"),
        std::path::PathBuf::from("/origin/.ae"),
    ];
    for slot in ["main", "worker.0", "spawned.1"] {
        let _ = std::hint::black_box(ae::meta::resolve_seat_dir(&parsed, slot));
        // Explicit rows refuse Missing where the typed resolve inherits.
        let _ = std::hint::black_box(ae::meta::explicit_seat_row(&parsed, slot));
        let typed =
            ae::meta::resolve_seat_target(&parsed, slot, std::path::PathBuf::from("/session"));
        let _ = std::hint::black_box(&typed);
        if let Ok(target) = typed {
            for root in &roots {
                let _ = std::hint::black_box(ae::meta::contained_in(&target.canonical, root));
            }
        }
    }
});
