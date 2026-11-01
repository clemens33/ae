#![no_main]

use ae::stats::{Scoped, Source, ThisSession, fold};
use libfuzzer_sys::fuzz_target;

const S: &str = "11111111-1111-4111-8111-111111111111";
const T: &str = "22222222-2222-4222-8222-222222222222";

/// Up to four sources, split at NUL. The third part is a meta, not a journal,
/// and that source replays the first journal under whatever identity it proves.
fn sources(data: &[u8]) -> Vec<Source> {
    let parts: Vec<&[u8]> = data.split(|byte| *byte == 0).take(4).collect();
    let first = parts.first().copied().unwrap_or_default();
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| match index {
            0 => Source::live(
                "a",
                format!("session_id={S}\n").as_bytes(),
                Ok(part.to_vec()),
            ),
            1 => Source::archive(T, Ok(part.to_vec())),
            2 => Source::live("c", part, Ok(first.to_vec())),
            _ => Source::archive(S, Ok(part.to_vec())),
        })
        .collect()
}

fn balanced(scoped: &Scoped) {
    assert!(scoped.balanced(), "{scoped:?}");
}

// Journal bytes are hostile: any can sit in a live session or a kept archive.
// Every table sums to the total, in both scopes, and the order sources were
// read in is no evidence.
fuzz_target!(|data: &[u8]| {
    let view = fold(sources(data), Some(S));
    balanced(&view.all);
    let ThisSession::Ready(current) = &view.this else {
        panic!("a session id was given");
    };
    balanced(current);
    assert!(current.total <= view.all.total);
    let reversed = fold(sources(data).into_iter().rev().collect(), Some(S));
    assert_eq!(view, reversed);
    let _ = std::hint::black_box(view.all.gaps.iter().map(|gap| gap.line()).count());
});
