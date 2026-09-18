#![no_main]

use libfuzzer_sys::fuzz_target;

/// The conversation id every fuzz read asks for. The documents that reach the
/// parser carry it in `info.id`; a mutation of that field exercises the
/// mismatch refusal instead.
const SID: &str = "ses_fuzzseatsession00000000000";

fuzz_target!(|data: &[u8]| {
    // The FIRST byte is the `--assistant` flag (`& 1`); the rest is one whole
    // export document. There is deliberately NO chunk-size byte: this reader
    // has no splitter — an export is ONE JSON document, never JSONL.
    let (assistant, export) = match data.split_first() {
        Some((flag, rest)) => (flag & 1 == 1, rest),
        None => (false, data),
    };
    let (rows, coverage) = ae::board::opencode::read(
        export,
        SID,
        "fuzz:opencode",
        ae::tool::ToolKind::OpenCode,
        assistant,
    );
    let _ = std::hint::black_box(ae::board::collect(rows));
    let _ = std::hint::black_box(coverage);
});
